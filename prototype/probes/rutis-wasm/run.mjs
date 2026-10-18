/**
 * Hosts the rutis-on-wasm probe.
 *
 * The module is a WASI reactor: no `_start`, so the host calls `initialize`
 * and then drives the exported `boot`. Everything the module wants from the
 * outside is standard WASI preview1 plus one `env.host_log`.
 *
 * Usage: node run.mjs [wasm32-wasip1|wasm32-unknown-unknown]
 */
import fs from 'node:fs'
import { WASI } from 'node:wasi'

const target = process.argv[2] ?? 'wasm32-wasip1'
const bytes = fs.readFileSync(`target/${target}/release/rutiswasm.wasm`)

let instance

const wasi = new WASI({ version: 'preview1', args: [], env: {} })

const imports = {
  wasi_snapshot_preview1: wasi.wasiImport,
  env: {
    host_log: (ptr, len) => {
      const view = new Uint8Array(instance.exports.memory.buffer, ptr, len)
      process.stdout.write(Buffer.from(view).toString('utf8') + '\n')
    },
  },
}

;({ instance } = await WebAssembly.instantiate(bytes, imports))

wasi.initialize(instance)

const responsePtr = instance.exports.boot()
const length = new DataView(instance.exports.memory.buffer).getUint32(responsePtr, true)
const body = Buffer.from(
  new Uint8Array(instance.exports.memory.buffer, responsePtr + 4, length),
).toString('utf8')

console.log(`  [wasm -> host] result: ${body}`)
