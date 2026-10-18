/**
 * demo-02: the real `@deepseek-ai/dsh-tool-fs` plugin, unmodified, running on
 * the co-kernel with `fs` served by the Rust core.
 *
 * The plugin module is imported from `packages/fs/tool-fs/lib` exactly as the
 * harness ships it. Nothing about it is re-implemented here: same `inject`
 * list (`tools`, `fs`, `systemPrompt`, `workingDirectory`), same config schema,
 * same tool definitions. Only the service behind `ctx.fs` changed address.
 *
 *   node demo-02-real-plugin.mjs
 */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { FsError } from '../../packages/fs/fs/lib/index.js'

import { startCoKernel } from './src/co-kernel.js'
import { WasmKernel } from './src/wasm-kernel.js'

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.join(here, '..', '..')
const modulePath = path.join(here, '..', 'kernel', 'target', 'wasm32-wasip1', 'release', 'evobuddy_kernel.wasm')

// A small tree the tool will read through the wasm core.
const fixture = path.join(here, '.fixture')
fs.rmSync(fixture, { force: true, recursive: true })
fs.mkdirSync(path.join(fixture, 'src'), { recursive: true })
fs.writeFileSync(
  path.join(fixture, 'src', 'greet.ts'),
  [
    'export interface Greeter {',
    '  greet(name: string): string',
    '}',
    '',
    'export function makeGreeter(prefix: string): Greeter {',
    '  return { greet: (name) => `${prefix}, ${name}!` }',
    '}',
    '',
  ].join('\n'),
)
fs.writeFileSync(path.join(fixture, 'notes.md'), '# notes\n\nwritten for the demo\n')

const kernel = await WasmKernel.load({
  modulePath,
  preopens: { '/workspace': fixture },
  onLog: (entry) => console.log(`  [core] ${entry.message}`),
})
kernel.boot({ root: '/workspace', patterns: ['export function'] })

const co = await startCoKernel({ kernel })

// Load the real plugin, restricted to the read tool.
const toolFs = await import(path.join(repoRoot, 'packages', 'fs', 'tool-fs', 'lib', 'index.js'))
await co.load(toolFs, { read: true, readImage: false, write: false, edit: false })
console.log('loaded packages/fs/tool-fs unmodified (read only)')

const read = co.ctx.tools.get('read')
assert.ok(read, 'the read tool is registered in the real ToolRuntime')
console.log('  registered tools:', JSON.stringify([...co.ctx.tools.view().visible.keys()]))
console.log('  system prompt mentions the tool:', co.ctx.systemPrompt != null)

// A fabricated execution: the registry pipeline is not under test here, the
// tool body and its `ctx.fs` service are.
const exec = {
  callId: 'demo-read-1',
  name: 'read',
  arguments: {},
  signal: new AbortController().signal,
}

// 1. read the whole file. The trailing newline does not start a new line, so
// the tool reports 7.
const whole = await read.execute({ file_path: 'src/greet.ts' }, exec)
assert.equal(whole.totalLines, 7)
assert.equal(whole.lines[0].number, 1)
assert.equal(whole.lines[0].text, 'export interface Greeter {')
assert.ok(whole.path.startsWith('/workspace/'), `path is canonical in the core's world: ${whole.path}`)
assert.equal(whole.path, '/workspace/src/greet.ts')
console.log(`  read ${whole.path}: ${whole.totalLines} lines`)

// 2. offset/limit windows.
const window = await read.execute({ file_path: 'src/greet.ts', offset: 5, limit: 2 }, exec)
assert.deepEqual(window.lines.map((line) => line.number), [5, 6])
assert.equal(window.lines[1].text, '  return { greet: (name) => `${prefix}, ${name}!` }')
console.log(`  window offset=5 limit=2 -> lines [${window.lines.map((l) => l.number).join(', ')}]`)

// 3. dsh error taxonomy survives the ABI: absence is FS_NOT_FOUND.
await assert.rejects(
  read.execute({ file_path: 'src/absent.ts' }, exec),
  (error) => error instanceof FsError && error.code === 'FS_NOT_FOUND',
)
console.log('  absent file -> FsError FS_NOT_FOUND (mapped from the kernel code)')

// 4. confinement holds at the tool boundary too. The escape is stopped twice:
// first by the WASI capability layer (ENOTCAPABLE), and a path that somehow
// canonicalized would still fail the kernel's prefix check. Either way the
// host sees one honest permission failure.
await assert.rejects(
  read.execute({ file_path: '../../../etc/hostname' }, exec),
  (error) => error instanceof FsError && error.code === 'FS_PERMISSION_DENIED',
)
console.log('  escape attempt -> FsError FS_PERMISSION_DENIED (WASI capability layer refused)')

// 5. both core capabilities serve the same tree: the tool read the file, and
// the search engine greps it in process. The pattern set lives in the provider
// plugin, so switching what "search" means is a hot-swap, not a rebuild.
const states = kernel.request('plugins.reload', { patterns: ['Greeter'] })
assert.ok(states.every((state) => state.state === 'Active'))
const grep = kernel.request('search.find', { root: '/workspace' })
assert.ok(grep.totalMatches >= 3, 'search sees the same content the tool read')
console.log(`  hot-swapped patterns -> search finds ${grep.totalMatches} matches for "Greeter" ` +
  `in ${grep.filesScanned} files, ${grep.elapsedMs.toFixed(1)}ms, no subprocess`)

await co.ctx.fiber.dispose()
kernel.shutdown()
fs.rmSync(fixture, { force: true, recursive: true })
console.log('\ndemo-02 PASS')
