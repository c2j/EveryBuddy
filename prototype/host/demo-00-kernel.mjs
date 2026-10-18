/**
 * demo-00: the kernel ABI end to end, no Cordis involved.
 *
 * Boots the wasm core in Node, drives every method the core exposes, and checks
 * the answers. This is the smallest proof that the Rust core is real and the
 * host can hold a conversation with it:
 *
 *   node demo-00-kernel.mjs
 */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { KernelError, WasmKernel } from './src/wasm-kernel.js'

const here = path.dirname(fileURLToPath(import.meta.url))
const modulePath = path.join(here, '..', 'kernel', 'target', 'wasm32-wasip1', 'release', 'evobuddy_kernel.wasm')

// A fixture the core reaches through WASI preopens, not through a host call.
const fixture = path.join(here, '.fixture')
fs.rmSync(fixture, { force: true, recursive: true })
fs.mkdirSync(path.join(fixture, 'src'), { recursive: true })
fs.writeFileSync(path.join(fixture, 'a.txt'), 'alpha\nneedle here\n')
fs.writeFileSync(path.join(fixture, 'src', 'b.rs'), 'fn b() // another needle\n')
fs.writeFileSync(path.join(fixture, 'c.bin'), Buffer.from([0, 1, 2, 0x6e, 0x65, 0x65, 0x64, 0x6c, 0x65, 0]))

const logs = []
const kernel = await WasmKernel.load({
  modulePath,
  preopens: { '/fixture': fixture },
  onLog: (entry) => logs.push(entry),
})

console.log(`kernel module version ${kernel.version}`)

// 0. a request before boot is a structured rejection, not a crash.
assert.throws(() => kernel.request('kernel.hello'), (error) => error.code === 'not_booted')
console.log('  pre-boot request rejected with not_booted')

// 1. boot loads the fs provider, the search gate, the search provider, and the
// consumer.
const booted = kernel.boot({ root: '/fixture', patterns: ['needle'] })
assert.equal(booted.replaced, false)
assert.equal(booted.plugins.length, 4)
console.log('  booted plugins:', booted.plugins.map((p) => `${p.name}@${p.id}:${p.state}`).join(', '))

// 2. hello describes the core.
const hello = kernel.request('kernel.hello')
assert.equal(hello.runtime, 'rutis')
assert.equal(hello.version, '0.1.0')
assert.equal(hello.root, '/fixture')
console.log('  hello:', JSON.stringify(hello))

// 3. a search goes through the rutis registry to the Rust engine.
const found = kernel.request('search.find', { root: '/fixture' })
assert.equal(found.totalMatches, 2)
assert.equal(found.filesScanned, 2, 'the binary file is skipped, not reported')
assert.deepEqual(found.patterns, ['needle'])
console.log(`  search: ${found.totalMatches} matches in ${found.filesScanned} files, ${found.elapsedMs.toFixed(1)}ms`)
console.log('  first hit:', JSON.stringify(found.hits[0]))

// 4. hot-plug: replacing the provider re-runs the dependent consumer.
const reloaded = kernel.request('plugins.reload', { patterns: ['alpha'] })
for (const state of reloaded) {
  assert.equal(state.state, 'Active')
  assert.notEqual(state.id, booted.plugins.find((p) => p.name === state.name).id, `${state.name} is a new fiber`)
}
const reports = kernel.request('search.reports')
assert.equal(reports.length, 2, 'one sweep per provider generation')
assert.deepEqual(reports[1].patterns, ['alpha'])
assert.equal(reports[1].totalMatches, 1)
console.log('  reload:', reloaded.map((p) => `${p.name}@${p.id}`).join(', '), `→ ${reports.length} sweeps`)

// 5. rejections are structured, so the host can branch on them.
for (const [method, params, code] of [
  ['does.not.exist', {}, 'unknown_method'],
  ['plugins.reload', { patterns: [] }, 'bad_request'],
  // 逃逸在 fs.resolve 一层就被拒(检索根与 fs 同一个路径世界);闸门仍在链上兜底。
  // preopen 之内不存在的路径是 NotFound;穿越到 preopen 之上才是逃逸。
  ['search.find', { root: '/fixture/../..' }, 'fs_outside_root'],
]) {
  assert.throws(() => kernel.request(method, params), (error) => error instanceof KernelError && error.code === code)
}
console.log('  rejections carry stable codes, including a confined-root refusal')

// 6. shutdown closes the core for real.
kernel.shutdown()
assert.throws(() => kernel.request('kernel.hello'), (error) => error.code === 'not_booted')
console.log('  shutdown: subsequent requests answer not_booted')

console.log(`\ncore log (${logs.length} entries):`)
for (const entry of logs) console.log(`  [${entry.level}] ${entry.message}`)

fs.rmSync(fixture, { force: true, recursive: true })
console.log('\ndemo-00 PASS')
