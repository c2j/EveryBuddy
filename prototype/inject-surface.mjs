#!/usr/bin/env node
/** What service surface do plugins actually depend on? Aggregates `inject` names. */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const report = JSON.parse(fs.readFileSync(path.join(HERE, 'report.json'), 'utf8')).report
const pluginSide = report.filter((r) => !r.empty && r.hostFiles > 0 && !r.authority)

const freq = new Map()
const per = new Map()
for (const r of pluginSide) {
  for (const name of r.inject) {
    if (name.startsWith('provides:')) continue
    freq.set(name, (freq.get(name) ?? 0) + 1)
    if (!per.has(name)) per.set(name, [])
    per.get(name).push(r.id)
  }
}
const total = pluginSide.length
console.log(`distinct injected service names: ${freq.size}`)
console.log(`plugin-side host packages:       ${total}\n`)
console.log('service'.padEnd(30) + 'packages  share')
for (const [name, count] of [...freq].sort((a, b) => b[1] - a[1])) {
  console.log(`${name.padEnd(30)} ${String(count).padStart(6)}   ${((count / total) * 100).toFixed(1)}%`)
}
const isolated = freq.size - [...freq].filter(([, c]) => c > 1).length
console.log(`\nservices injected by exactly one package: ${isolated}`)
