/**
 * Loads and drives the EvoBuddy kernel wasm module.
 *
 * The whole surface is five exports plus one import, mirrored by five methods
 * plus one dispatcher here. Everything crossing is a framed JSON buffer in the
 * core's linear memory: the host writes a request where the core can read it,
 * the core answers the same way, and both sides free what they read.
 *
 * Calling is synchronous by construction. The core advances only while a request
 * is being served, so a reply means the work finished, not that a task was
 * scheduled.
 */
import fs from 'node:fs'
import { WASI } from 'node:wasi'

/** Bytes of length prefix ahead of every framed payload. */
const PREFIX = 4

/** A rejection from the core, carrying its stable code. */
export class KernelError extends Error {
  /**
   * @param {string} code stable identifier the host can branch on
   * @param {string} message human-readable explanation from the core
   */
  constructor(code, message) {
    super(message)
    this.name = 'KernelError'
    this.code = code
  }
}

/**
 * One loaded kernel instance.
 *
 * An instance is single-use: a trap ends it, and the caller builds a new one
 * rather than reusing a core whose state is unknown.
 */
export class WasmKernel {
  /** @type {WebAssembly.Instance|null} */
  #instance = null
  /** @type {import('node:wasi').WASI|null} */
  #wasi = null
  /** @type {(entry: {level: string, message: string}) => void} */
  #onLog
  /** @type {number} */
  #nextId = 1

  /**
   * @param {(entry: {level: string, message: string}) => void} onLog
   */
  constructor(onLog) {
    this.#onLog = onLog
  }

  /**
   * Instantiate the module and run its WASI start-up.
   *
   * @param {object} options
   * @param {string} options.modulePath compiled `evobuddy_kernel.wasm`
   * @param {Record<string, string>} [options.preopens] guest path -> host directory
   * @param {(entry: {level: string, message: string}) => void} [options.onLog]
   * @returns {Promise<WasmKernel>}
   */
  static async load({ modulePath, preopens = {}, onLog = () => {} }) {
    const bytes = fs.readFileSync(modulePath)
    const kernel = new WasmKernel(onLog)
    const wasi = new WASI({ version: 'preview1', args: ['evobuddy-kernel'], env: {}, preopens })
    const imports = {
      wasi_snapshot_preview1: wasi.wasiImport,
      env: {
        host_call: (ptr, len) => kernel.#hostCall(ptr, len),
      },
    }
    const { instance } = await WebAssembly.instantiate(bytes, imports)
    kernel.#instance = instance
    kernel.#wasi = wasi
    wasi.initialize(instance)
    return kernel
  }

  /** Packed semantic version of the loaded core. */
  get version() {
    const packed = this.#exports.kernel_version()
    return `${(packed >> 16) & 0xff}.${(packed >> 8) & 0xff}.${packed & 0xff}`
  }

  /** @returns {WebAssembly.Exports} */
  get #exports() {
    if (!this.#instance) throw new KernelError('not_loaded', 'WasmKernel.load has not run')
    return this.#instance.exports
  }

  /**
   * Build the core's runtime and plugin pair.
   *
   * @param {{root: string, patterns: string[], limits?: object}} config
   * @returns {{replaced: boolean, plugins: Array<object>}}
   */
  boot(config) {
    const response = this.#call('kernel_boot', JSON.stringify(config))
    return this.#unwrap(response, null)
  }

  /**
   * Route one request.
   *
   * @param {string} method dotted method name from `kernel.hello.methods`
   * @param {object} [params]
   * @returns {unknown} the core's result payload
   * @throws {KernelError} when the core rejects the request
   */
  request(method, params = {}) {
    const id = this.#nextId++
    const response = this.#call('kernel_request', JSON.stringify({ id, method, params }))
    return this.#unwrap(response, id)
  }

  /** Shut the core down. Later requests answer `not_booted`. */
  shutdown() {
    this.#exports.kernel_shutdown()
  }

  /**
   * Write one request and read one reply.
   *
   * @param {'kernel_boot'|'kernel_request'} exportName
   * @param {string} json request payload
   * @returns {object} parsed response envelope
   */
  #call(exportName, json) {
    const bytes = Buffer.from(json, 'utf8')
    const inPtr = this.#exports.kernel_alloc(bytes.length)
    try {
      // Re-read the buffer after the allocation: growing linear memory detaches
      // the previous ArrayBuffer.
      new Uint8Array(this.#exports.memory.buffer, inPtr, bytes.length).set(bytes)
      const outPtr = this.#exports[exportName](inPtr, bytes.length)
      return JSON.parse(this.#readFrame(outPtr).toString('utf8'))
    } finally {
      this.#exports.kernel_free(inPtr, bytes.length)
    }
  }

  /**
   * Validate a response envelope and return its result.
   *
   * @param {object} response
   * @param {number|null} expectedId id the caller sent, or `null` when the
   *   core generated the id itself (boot)
   * @returns {unknown}
   */
  #unwrap(response, expectedId) {
    if (expectedId !== null && response.id !== expectedId) {
      throw new KernelError('id_mismatch', `core answered request ${response.id}, sent ${expectedId}`)
    }
    if (!response.ok) {
      throw new KernelError(response.error.code, response.error.message)
    }
    return response.result
  }

  /**
   * Read a framed buffer the core handed out and release it.
   *
   * @param {number} ptr
   * @returns {Buffer}
   */
  #readFrame(ptr) {
    const view = new DataView(this.#exports.memory.buffer)
    const length = view.getUint32(ptr, true)
    const body = Buffer.from(new Uint8Array(this.#exports.memory.buffer, ptr + PREFIX, length))
    this.#exports.kernel_free(ptr, PREFIX + length)
    return body
  }

  /**
   * Answer one `host_call` from the core.
   *
   * @param {number} ptr guest pointer to the framed call payload
   * @param {number} len
   * @returns {number} guest pointer to a framed reply the core will free
   */
  #hostCall(ptr, len) {
    // A plain Uint8Array's toString() joins decimal bytes; only Buffer decodes.
    const payload = JSON.parse(Buffer.from(new Uint8Array(this.#exports.memory.buffer, ptr, len)).toString('utf8'))
    let reply = { ok: true, result: null }
    try {
      switch (payload.method) {
        case 'log':
          this.#onLog(payload.params)
          break
        default:
          reply = { ok: false, result: null, error: `host has no method: ${payload.method}` }
      }
    } catch (error) {
      reply = { ok: false, result: null, error: error instanceof Error ? error.message : String(error) }
    }
    const body = Buffer.from(JSON.stringify(reply), 'utf8')
    const outPtr = this.#exports.kernel_alloc(PREFIX + body.length)
    // Allocation may have grown memory, so read the buffer fresh.
    const view = new DataView(this.#exports.memory.buffer)
    view.setUint32(outPtr, body.length, true)
    new Uint8Array(this.#exports.memory.buffer, outPtr + PREFIX, body.length).set(body)
    return outPtr
  }
}
