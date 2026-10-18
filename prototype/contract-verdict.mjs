#!/usr/bin/env node
/**
 * Turns contract.json into the decision the whole exercise rests on.
 *
 * For every Host-side `service.member` pair it asks two questions:
 *   1. Can the member be asynchronous?  (declared async, or every call site awaits)
 *   2. If not, does it hand back a live object?  (its return type names a class
 *      declared in this repository, so callers depend on object identity and
 *      methods, not on a copyable value)
 *
 * "async OK"       -> a kernel in another process can serve it over a bridge
 * "mirrorable"     -> synchronous, but the answer is plain data a JS-side cache
 *                     can hold
 * "in-process"     -> synchronous and returns a live object; the owner must
 *                     share a process with its callers
 *
 * Usage: node contract-verdict.mjs [repoRoot]
 */
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(process.argv[2] ?? path.join(HERE, '..'))
const require = createRequire(path.join(ROOT, 'package.json'))
const ts = require('typescript')

function listSources(dir) {
  const out = []
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, entry.name)
      if (entry.isDirectory()) walk(p)
      else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith('.d.ts')) out.push(p)
    }
  }
  const src = path.join(dir, 'src')
  if (fs.existsSync(src)) walk(src)
  return out
}

/** Every class declared anywhere in packages/, for the identity test. */
const classNames = new Set()
const packagesDir = path.join(ROOT, 'packages')
for (const group of fs.readdirSync(packagesDir)) {
  const groupDir = path.join(packagesDir, group)
  if (!fs.statSync(groupDir).isDirectory()) continue
  for (const dir of fs.readdirSync(groupDir)) {
    const pkgDir = path.join(groupDir, dir)
    if (!fs.existsSync(path.join(pkgDir, 'package.json'))) continue
    for (const file of listSources(pkgDir)) {
      const text = fs.readFileSync(file, 'utf8')
      if (!/\bclass\s/.test(text)) continue
      const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
      const visit = (node) => {
        if ((ts.isClassDeclaration(node) || ts.isClassExpression(node)) && node.name) classNames.add(node.name.text)
        ts.forEachChild(node, visit)
      }
      visit(sf)
    }
  }
}

const rows = JSON.parse(fs.readFileSync(path.join(HERE, 'contract.json'), 'utf8'))
const total = (r) => r.calls.awaited + r.calls.asyncFn + r.calls.syncFn + r.calls.fieldInit + r.calls.topLevel
const ranked = [...rows].sort((a, b) => total(b) - total(a))

const liveType = (type) => {
  if (!type) return false
  if (/^\(\s*\)\s*=>/.test(type) || /\bDisposable\b/.test(type)) return true
  const names = type.match(/[A-Z][A-Za-z0-9_]*/g) ?? []
  return names.some((n) => classNames.has(n))
}

const classify = (r) => {
  if (r.asyncDeclared === true) return 'asyncOK'
  if (r.asyncDeclared === null) return 'unknown'
  if (r.syncSites === 0) return 'asyncOK' // declared sync but every caller awaits
  const t = r.type ?? ''
  if (/^\(\s*\)\s*=>/.test(t) || /\bDisposable\b/.test(t)) return 'syncRegister'
  return liveType(r.type) ? 'syncLive' : 'syncData'
}

for (const r of ranked) r.verdict = classify(r)

const EMPTY = () => ({ asyncOK: 0, syncRegister: 0, syncLive: 0, syncData: 0, unknown: 0 })
const tally = (list) => {
  const out = EMPTY()
  for (const r of list) out[r.verdict]++
  return out
}

console.log(`distinct classes in packages/      ${classNames.size}`)
console.log(`Host-side pairs analysed           ${rows.length}\n`)
console.log('slice'.padEnd(20) + 'asyncOK  sync-register  sync-live  sync-data  unknown')
for (const [label, n] of [['top 33 by calls', 33], ['top 135 by calls', 135], ['all Host pairs', rows.length]]) {
  const t = tally(ranked.slice(0, n))
  console.log(label.padEnd(20) + String(t.asyncOK).padStart(7) + String(t.syncRegister).padStart(15) + String(t.syncLive).padStart(11) + String(t.syncData).padStart(11) + String(t.unknown).padStart(9))
}

const kernels = new Set(['logger', 'fiber', 'events', 'reflect', 'registry'])
const dshOnly = ranked.filter((r) => !kernels.has(r.service))
console.log('\nexcluding the 5 Cordis runtime services:')
for (const [label, n] of [['top 33 by calls', 33], ['top 135 by calls', 135], ['all Host pairs', dshOnly.length]]) {
  const t = tally(dshOnly.slice(0, n))
  console.log(label.padEnd(20) + String(t.asyncOK).padStart(7) + String(t.syncRegister).padStart(15) + String(t.syncLive).padStart(11) + String(t.syncData).padStart(11) + String(t.unknown).padStart(9))
}

// per-service split: which services are async-friendly and which are synchronous
const perService = new Map()
for (const r of ranked) {
  if (!perService.has(r.service)) perService.set(r.service, { asyncOK: 0, sync: 0, unknown: 0, calls: 0, members: 0, provider: r.provider })
  const e = perService.get(r.service)
  e.members++
  e.calls += total(r)
  if (r.verdict === 'asyncOK') e.asyncOK++
  else if (r.verdict === 'unknown') e.unknown++
  else e.sync++
}
const services = [...perService].filter(([, e]) => e.calls >= 3).sort((a, b) => b[1].calls - a[1].calls)
console.log('\n--- per service: async vs synchronous members (host side, >=3 calls) ---')
console.log('service'.padEnd(20) + 'calls  members  async  sync  unknown  character')
for (const [service, e] of services) {
  const known = e.members - (e.unknown ?? 0)
  const character = known === 0 ? 'unresolved'
    : e.sync === 0 ? 'all async'
    : e.asyncOK === 0 ? 'all sync'
    : 'mixed'
  console.log(`${service.padEnd(20)} ${String(e.calls).padStart(5)} ${String(e.members).padStart(8)} ${String(e.asyncOK).padStart(6)} ${String(e.sync).padStart(5)} ${String(e.unknown ?? 0).padStart(8)}  ${character}`)
}

console.log('\n--- pairs whose provider class was not found in packages/ ---')
for (const r of ranked.filter((r) => r.verdict === 'unknown')) {
  console.log(`  ${r.pair.padEnd(34)} calls=${String(total(r)).padStart(4)}`)
}

// Which package owns the members that must answer without an await?
const byProvider = new Map()
for (const r of ranked) {
  if (r.verdict !== 'syncRegister' && r.verdict !== 'syncLive' && r.verdict !== 'syncData') continue
  const key = r.provider ?? `(none: ${r.service})`
  if (!byProvider.has(key)) byProvider.set(key, [])
  byProvider.get(key).push(r)
}
console.log('\n--- synchronous members grouped by providing package (the in-process set) ---')
for (const [provider, list] of [...byProvider].sort((a, b) => b[1].length - a[1].length)) {
  const verdicts = list.map((r) => r.verdict)
  const rank = (v) => (v === 'syncLive' ? 0 : v === 'syncData' ? 1 : 2)
  const worst = [...verdicts].sort((a, b) => rank(a) - rank(b))[0]
  console.log(`${provider.padEnd(26)} members=${String(list.length).padStart(3)}  worst=${worst.padEnd(12)} ${list.map((r) => r.member).join(' ')}`)
}

const group = (verdict, label, limit) => {
  const list = ranked.filter((r) => r.verdict === verdict)
  console.log(`\n--- ${label}: ${list.length} pairs ---`)
  for (const r of list.slice(0, limit)) {
    console.log(`  ${r.pair.padEnd(36)} calls=${String(total(r)).padStart(4)}  ${(r.type ?? '(no type)').slice(0, 52)}`)
  }
}
group('syncLive', 'synchronous, returns a named/live type (JS mirror would hand back a stale handle)', 45)
group('syncRegister', 'synchronous registration returning a disposer', 30)
group('syncData', 'synchronous, plain data', 30)
group('asyncOK', 'already asynchronous — a bridge can serve it', 40)

fs.writeFileSync(path.join(HERE, 'contract-verdict.json'), JSON.stringify({
  classNames: [...classNames].sort(),
  rows: ranked.map((r) => ({ ...r, calls: undefined, verdict: r.verdict, totalCalls: total(r) })),
}, null, 1) + '\n')
console.log(`\nfull report: ${path.join(HERE, 'contract-verdict.json')}`)
