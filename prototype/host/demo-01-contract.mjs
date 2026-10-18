/**
 * demo-01: a dsh-shaped contract plugin, loaded by real vendored Cordis, served
 * by the Rust core.
 *
 * Proves, in order:
 *   1. the co-kernel boots real Cordis with real SystemPrompt and ToolRuntime;
 *   2. a plugin that injects `evoSearch` runs and receives real search results
 *      produced inside the wasm core;
 *   3. injection is real: a context without `evoSearch` leaves the same plugin
 *      pending instead of calling `apply`;
 *   4. the hot-swap path (`evoSearch.use`) re-runs a fresh provider generation
 *      without touching the Node side.
 *
 *   node demo-01-contract.mjs
 */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { Context } from '../../vendor/cordis/lib/index.js'

import { startCoKernel } from './src/co-kernel.js'
import { WasmKernel } from './src/wasm-kernel.js'
import * as contract from './contracts/evo-search-contract.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const modulePath = path.join(here, '..', 'kernel', 'target', 'wasm32-wasip1', 'release', 'evobuddy_kernel.wasm')

// The tree the contract plugin searches: this repository's own kernel sources,
// so the hits are real content and not a planted fixture.
const searchRoot = path.join(here, '..', 'kernel', 'src')

const kernel = await WasmKernel.load({
  modulePath,
  // The preopen is the confinement: the core cannot name paths outside it.
  preopens: { '/workspace': searchRoot },
  onLog: (entry) => console.log(`  [core] ${entry.message}`),
})
kernel.boot({ root: '/workspace', patterns: ['pub fn', 'unsafe'] })
console.log('co-kernel: Rust core booted on the kernel sources')

const reports = []
const co = await startCoKernel({
  kernel,
  mount: (ctx) => ctx.on('evo/contract-report', (report) => reports.push(report)),
})

// 1. the real services are up.
assert.ok(co.ctx.systemPrompt, 'SystemPrompt service is mounted')
assert.ok(co.ctx.tools, 'ToolRuntime service is mounted')
console.log('  cordis services: systemPrompt, tools mounted for real')

// 2. the contract plugin runs against the Rust core.
await co.load(contract, { root: '/workspace' })
assert.equal(reports.length, 1, 'apply ran exactly once')
const report = reports[0]
assert.ok(report.totalMatches > 0, 'the search found real content')
assert.ok(report.firstHit.path.endsWith('.rs'), `hits come from the mounted tree: ${report.firstHit.path}`)
assert.ok(report.firstHit.text.length > 0, 'hits carry the matching line')
console.log(`  contract plugin: ${report.totalMatches} matches for [${report.patterns.join(', ')}] ` +
  `in ${report.filesScanned} files (core ${report.coreElapsedMs.toFixed(1)}ms, round trip ${report.hostRoundTripMs}ms)`)
console.log(`  first hit: ${report.firstHit.path}:${report.firstHit.line} — ${report.firstHit.text.slice(0, 60)}`)

// 3. injection is load-bearing. A context without evoSearch must leave the same
// plugin pending, never call apply with a half-resolved context.
const bare = new Context()
const bareReports = []
bare.on('evo/contract-report', (r) => bareReports.push(r))
const pending = bare.plugin(contract, { root: '/workspace' })
await new Promise((resolve) => setTimeout(resolve, 50))
assert.notEqual(pending.state, 2, 'the fiber must not be active without its dependency')
assert.equal(bareReports.length, 0, 'apply must not run without evoSearch')
console.log(`  without evoSearch the plugin stays pending (fiber state ${pending.state}), apply never ran`)
await bare.fiber.dispose()

// 4. hot-swap: new patterns, new provider generation, same plugin contract.
const states = co.ctx.evoSearch.use(['FsService', 'thread_local'])
assert.ok(states.every((state) => state.state === 'Active'), 'reload leaves every plugin active')
const after = kernel.request('search.reports')
assert.equal(after.length, 2, 'a second sweep ran for the new provider')
assert.deepEqual(after[1].patterns, ['FsService', 'thread_local'])
console.log(`  hot-swap: provider replaced, ${after[1].totalMatches} matches for the new patterns`)

// The same contract plugin still resolves after the swap, because the service
// identity lives in the registry, not in a captured reference.
reports.length = 0
await co.load(contract, { root: '/workspace' })
assert.equal(reports.length, 1, 'the plugin still runs after a core-side reload')
assert.deepEqual(reports[0].patterns, ['FsService', 'thread_local'])
console.log('  contract plugin reloaded cleanly against the new provider generation')

await co.ctx.fiber.dispose()
kernel.shutdown()
console.log('\ndemo-01 PASS')
