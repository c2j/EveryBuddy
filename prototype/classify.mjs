#!/usr/bin/env node
/**
 * Applies a symbol-level classification to scan.mjs's report.json.
 *
 * `unsafe.json` lists authority symbols whose state would live in the Rust
 * kernel. A plugin that value-imports one of them cannot run on a facade alone.
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const report = JSON.parse(fs.readFileSync(path.join(HERE, 'report.json'), 'utf8'))
const UNSAFE = new Set(JSON.parse(fs.readFileSync(path.join(HERE, 'unsafe.json'), 'utf8')))

const pluginSide = report.report.filter((r) => !r.empty && r.hostFiles > 0 && !r.authority)
const n = pluginSide.length

const reasons = { unsafeSymbol: [], waterfall: [], sessionDirect: [], namespace: [] }
const blocked = new Set()
for (const r of pluginSide) {
  const hits = r.authorityValues.filter((v) => UNSAFE.has(v.split('#')[0] + '#' + v.split('#')[1]))
  if (hits.length) reasons.unsafeSymbol.push([r.id, hits])
  if (r.waterfall > 0) reasons.waterfall.push([r.id, r.waterfall])
  if (r.sessionDirect > 0) reasons.sessionDirect.push([r.id, r.sessionDirect])
  if (hits.length || r.waterfall > 0 || r.sessionDirect > 0) blocked.add(r.id)
}

const viable = pluginSide.filter((r) => !blocked.has(r.id))
const pct = (x) => `${((x / n) * 100).toFixed(1)}%`

console.log(`plugin-side host packages            ${n}`)
console.log(`  blocked by an authority symbol     ${new Set(reasons.unsafeSymbol.map((x) => x[0])).size}`)
console.log(`  blocked by waterfall               ${reasons.waterfall.length}`)
console.log(`  blocked by direct ctx.session      ${reasons.sessionDirect.length}`)
console.log(`  BLOCKED (union)                    ${blocked.size}   ${pct(blocked.size)}`)
console.log(`  RUN UNCHANGED on a Rust kernel     ${viable.length}   ${pct(viable.length)}`)

console.log('\n--- blocked by an authority symbol ---')
for (const [id, hits] of reasons.unsafeSymbol.sort()) console.log(`  ${id.padEnd(52)} ${hits.join(', ')}`)
console.log('\n--- blocked by waterfall ---')
for (const [id, c] of reasons.waterfall) console.log(`  ${id.padEnd(52)} ${c} call site(s)`)
console.log('\n--- blocked by direct ctx.session access ---')
for (const [id, c] of reasons.sessionDirect) console.log(`  ${id.padEnd(52)} ${c} access(es)`)

console.log('\n--- authority symbol frequency across the blocked set ---')
const freq = new Map()
for (const [, hits] of reasons.unsafeSymbol) for (const h of hits) freq.set(h, (freq.get(h) ?? 0) + 1)
for (const [k, v] of [...freq].sort((a, b) => b[1] - a[1])) console.log(`  ${k.padEnd(58)} ${v}`)
