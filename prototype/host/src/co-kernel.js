/**
 * The Node co-kernel: real vendored Cordis hosting real dsh plugins, with two
 * services served by the Rust core through the wasm ABI.
 *
 * Division of labour, unchanged since the design:
 *
 * - Cordis owns plugin loading, injection, effects, and events. Nothing here
 *   re-implements any of it.
 * - The Rust core owns the capabilities it is uniquely good at: in-process
 *   multi-pattern search and confined filesystem reads over WASI preopens.
 * - This file is the seam: four adapters that present kernel calls as Cordis
 *   services, so a plugin cannot tell which side serves it.
 *
 * Every kernel call is synchronous (the wasm module advances only while a
 * request is being served), so adapters that must look async simply wrap the
 * synchronous answer in a promise, and `processPath` stays synchronous the way
 * the real `FileSystem` declares it.
 */
import { Context } from '../../../vendor/cordis/lib/index.js'
import { FsError } from '../../../packages/fs/fs/lib/index.js'
import { SystemPrompt } from '../../../packages/core/system-prompt/lib/index.js'
import { ToolRuntime } from '../../../packages/core/tools/lib/index.js'

/** Kernel failure code -> the `FsErrorCode` a dsh consumer expects. */
const FS_CODES = {
  fs_not_found: 'FS_NOT_FOUND',
  fs_not_regular_file: 'FS_NOT_REGULAR_FILE',
  fs_outside_root: 'FS_PERMISSION_DENIED',
  fs_too_large: 'FS_TOO_LARGE',
  fs_io_error: 'FS_IO_ERROR',
}

/**
 * Translate a kernel rejection into the error type dsh filesystem consumers
 * catch, preserving the message and the code class.
 * @param {Error & {code?: string}} error
 * @returns {FsError}
 */
function toFsError(error) {
  const code = (error instanceof Error && error.code in FS_CODES)
    ? FS_CODES[error.code]
    : 'FS_IO_ERROR'
  return new FsError(error instanceof Error ? error.message : String(error), code)
}

/**
 * The `ctx.fs` surface the read tool exercises, backed by the Rust core.
 * Members the demo does not exercise are deliberately absent: a caller that
 * reaches one fails loudly instead of silently getting a different backend.
 * @param {import('./wasm-kernel.js').WasmKernel} kernel
 */
function fsFromKernel(kernel) {
  const key = (target) => {
    if (typeof target?.targetKey !== 'string') {
      throw new FsError('resolve() produced this target; pass it unchanged', 'FS_IO_ERROR')
    }
    return { targetKey: target.targetKey }
  }
  return {
    /** @returns {Promise<{targetKey: string, displayPath: string}>} */
    async resolve(path, opts = {}) {
      try {
        return await kernel.request('fs.resolve', {
          path,
          ...(opts.cwd !== undefined ? { cwd: opts.cwd } : {}),
        })
      } catch (error) {
        throw toFsError(error)
      }
    },
    /** @returns {Promise<{version: string, type: string, size?: number} | undefined>} */
    async stat(target) {
      try {
        const info = await kernel.request('fs.stat', key(target))
        return info ?? undefined
      } catch (error) {
        throw toFsError(error)
      }
    },
    /** @returns {Promise<string>} */
    async readText(target) {
      try {
        return (await kernel.request('fs.readText', key(target))).content
      } catch (error) {
        throw toFsError(error)
      }
    },
    /**
     * Single-chunk stream. The core reads the file once; the tool only streams
     * when `size >= readStreamMinSize`, which no fixture in these demos reaches.
     */
    async streamText(target) {
      const content = await this.readText(target)
      return (async function* () {
        yield content
      })()
    },
    /** @returns {string} synchronous, as `FileSystem.processPath` declares */
    processPath(target) {
      try {
        return kernel.request('fs.processPath', key(target)).path
      } catch (error) {
        throw toFsError(error)
      }
    },
  }
}

/**
 * The `evoSearch` contract service: what a plugin sees when the Rust core
 * serves search. This is the contract demo-01 pins; a future implementation in
 * Node or in another core can replace it without touching the plugin.
 * @param {import('./wasm-kernel.js').WasmKernel} kernel
 */
function evoSearchFromKernel(kernel) {
  return {
    /** @returns {object} a `SearchOutcome` from the core */
    find(params = {}) {
      return kernel.request('search.find', params)
    },
    /** Hot-swap the pattern set; returns the new plugin states. */
    use(patterns) {
      return kernel.request('plugins.reload', { patterns })
    },
    /** One sweep report per provider generation, oldest first. */
    reports() {
      return kernel.request('search.reports')
    },
  }
}

/**
 * Boot the co-kernel.
 *
 * @param {object} options
 * @param {import('./wasm-kernel.js').WasmKernel} options.kernel the loaded Rust core
 * @param {(ctx: Context) => void} [options.mount] extra services to provide
 *   before plugins load; use `ctx.reflect.provide(name, value)` or `ctx.on`
 * @param {object} [options.workingDirectory] replacement for the stub
 *   `workingDirectory` service
 * @returns {Promise<{ctx: Context, load: (plugin: object, config?: object) => Promise<unknown>}>}
 */
export async function startCoKernel({ kernel, mount, workingDirectory }) {
  const ctx = new Context()

  // dsh services hosted for real, in dsh's own order: SystemPrompt first,
  // then ToolRuntime, which declares `inject: ['systemPrompt']`.
  await ctx.plugin(SystemPrompt, {})
  await ctx.plugin(ToolRuntime, { mode: 'native' })

  // Services the Rust core serves. `reflect.provide` ties each registration to
  // this plugin's fiber, exactly like a Service subclass constructor would.
  await ctx.plugin({
    name: 'kernel-services',
    apply(serviceCtx) {
      serviceCtx.reflect.provide('fs', fsFromKernel(kernel))
      serviceCtx.reflect.provide('evoSearch', evoSearchFromKernel(kernel))
      serviceCtx.reflect.provide(
        'workingDirectory',
        workingDirectory ?? { ensure: async () => undefined },
      )
    },
  })

  // Deployment-provided extras load last, so a plugin that injects them sees
  // the same context the demo plugins will.
  if (mount) mount(ctx)

  return {
    ctx,
    /** Load one plugin module into the co-kernel. */
    load(plugin, config) {
      return ctx.plugin(plugin, config)
    },
  }
}
