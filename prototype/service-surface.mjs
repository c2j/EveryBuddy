#!/usr/bin/env node
/**
 * Service-contract surface: which members of which service do plugins actually
 * call?
 *
 * The coupling scan measures direct `import` coupling. It cannot see the far
 * larger surface that runs through `inject` + `ctx.<service>`: a plugin that
 * injects `tools` and calls `tools.register(...)` depends on the full semantics
 * of `register` without importing anything.
 *
 * This script walks every plugin-side source file and resolves
 *
 *     ctx.<service>.<member>(...)          direct access
 *     ctx.get<...>('<service>').<member>   strict optional read
 *     const s = ctx.<service>; s.<member>  aliased access
 *     const { register } = ctx.tools       destructured member
 *
 * into a ranked `service -> member` table, per package. That table is the
 * interface a replacement kernel has to match member by member.
 *
 * Usage: node service-surface.mjs [repoRoot]
 */
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(process.argv[2] ?? path.join(HERE, '..'))
const require = createRequire(path.join(ROOT, 'package.json'))
const ts = require('typescript')

const AUTHORITY_DIRS = /^(core\/|llm\/|session\/|boot\/|api\/)/

function listPackages() {
  const out = []
  const packagesDir = path.join(ROOT, 'packages')
  for (const group of fs.readdirSync(packagesDir)) {
    const groupDir = path.join(packagesDir, group)
    if (!fs.statSync(groupDir).isDirectory()) continue
    for (const dir of fs.readdirSync(groupDir)) {
      const pkgDir = path.join(groupDir, dir)
      const manifest = path.join(pkgDir, 'package.json')
      if (!fs.existsSync(manifest)) continue
      out.push({ id: `${group}/${dir}`, dir: pkgDir, authority: AUTHORITY_DIRS.test(`${group}/${dir}`) })
    }
  }
  return out
}

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

const isCtxName = (name) => name === 'ctx' || /Ctx$/.test(name) || name === 'root'
const isClientFile = (pkg, file) =>
  pkg.id.startsWith('client/') || /(^|\/)client(\/|$)/.test(path.relative(pkg.dir, file)) || pkg.id.includes('-client-')

/** Resolves the context handle a node refers to, if any. */
function ctxBase(node) {
  if (ts.isIdentifier(node) && isCtxName(node.text)) return true
  if (ts.isPropertyAccessExpression(node) && node.name.text === 'ctx') return true
  return false
}

function analyse(pkg, file, sink) {
  const text = fs.readFileSync(file, 'utf8')
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const client = isClientFile(pkg, file)
  const toStringArg = (n) =>
    n && ts.isStringLiteral(n) ? n.text : n && ts.isArrayLiteralExpression(n) && n.elements.length === 1 && ts.isStringLiteral(n.elements[0]) ? n.elements[0].text : undefined

  // ---- pass 1: aliases that point at a service -------------------------
  const alias = new Map() // local name -> service name
  const destructured = [] // { service, member }
  const visitAliases = (node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
      const init = node.initializer
      if (ts.isPropertyAccessExpression(init) && ctxBase(init.expression)) alias.set(node.name.text, init.name.text)
      else if (ts.isCallExpression(init) && ts.isPropertyAccessExpression(init.expression) &&
        init.expression.name.text === 'get' && ctxBase(init.expression.expression)) {
        const svc = toStringArg(init.arguments[0])
        if (svc) alias.set(node.name.text, svc)
      }
    }
    // const { register, unregister } = ctx.tools
    if (ts.isVariableDeclaration(node) && ts.isObjectBindingPattern(node.name) && node.initializer &&
      ts.isPropertyAccessExpression(node.initializer) && ctxBase(node.initializer.expression)) {
      for (const el of node.name.elements) {
        const local = el.name
        if (ts.isIdentifier(local)) destructured.push({ service: node.initializer.name.text, member: local.text })
      }
    }
    ts.forEachChild(node, visitAliases)
  }
  visitAliases(sf)
  for (const d of destructured) sink(pkg.id, d.service, d.member, 'destructured', client)

  // ---- pass 2: member access through a service -------------------------
  const record = (service, member, kind) => sink(pkg.id, service, member, kind, client)
  const visit = (node) => {
    if (ts.isPropertyAccessExpression(node)) {
      const owner = node.expression
      const called = ts.isCallExpression(node.parent) && node.parent.expression === node
      const kind = called ? 'call' : 'read'
      if (ts.isPropertyAccessExpression(owner) && ctxBase(owner.expression)) record(owner.name.text, node.name.text, kind)
      else if (ts.isCallExpression(owner) && ts.isPropertyAccessExpression(owner.expression) &&
        owner.expression.name.text === 'get' && ctxBase(owner.expression.expression)) {
        const svc = toStringArg(owner.arguments[0])
        if (svc) record(svc, node.name.text, kind)
      } else if (ts.isIdentifier(owner) && alias.has(owner.text)) record(alias.get(owner.text), node.name.text, kind)
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
}

const packages = listPackages()
const injected = new Set()
const stats = new Map() // `${service}\u0000${member}` -> {...}
/** Cordis runtime services reached through `ctx`, not dsh services. */
const KERNEL_SERVICES = new Set(['logger', 'fiber', 'events', 'reflect', 'registry'])

const sink = (pkgId, service, member, kind, client) => {
  const key = `${service}\u0000${member}`
  if (!stats.has(key)) stats.set(key, { service, member, call: 0, read: 0, destructured: 0, host: new Set(), client: new Set() })
  const e = stats.get(key)
  e[kind] = (e[kind] ?? 0) + 1
  ;(client ? e.client : e.host).add(pkgId)
}
/** A service belongs to whichever side most of its touching packages live on. */
const sideOf = (rows) => {
  const host = new Set()
  const client = new Set()
  for (const r of rows) {
    for (const p of r.host) host.add(p)
    for (const p of r.client) client.add(p)
  }
  return host.size >= client.size ? 'host' : 'client'
}

for (const pkg of packages) {
  if (pkg.authority) continue
  const files = listSources(pkg.dir)
  if (files.length === 0) continue
  for (const file of files) analyse(pkg, file, sink)
}

// injected names, for marking which services are declared vs read optionally
const report = JSON.parse(fs.readFileSync(path.join(HERE, 'report.json'), 'utf8')).report
for (const r of report) {
  if (r.authority) continue
  for (const n of r.inject ?? []) if (!n.startsWith('provides:')) injected.add(n)
}

const rows = [...stats.values()].sort(
  (a, b) => b.host.size + b.client.size - (a.host.size + a.client.size) || b.call - a.call,
)
const byService = new Map()
for (const row of rows) {
  if (!byService.has(row.service)) byService.set(row.service, [])
  byService.get(row.service).push(row)
}
const nonKernel = [...byService].filter(([s]) => !KERNEL_SERVICES.has(s))
const split = { host: [], client: [] }
for (const entry of nonKernel) split[sideOf(entry[1])].push(entry)
for (const side of ['host', 'client']) {
  split[side].sort((a, b) => (b[1].reduce((n, r) => n + r.host.size + r.client.size, 0)) - (a[1].reduce((n, r) => n + r.host.size + r.client.size, 0)))
}

const pkgCount = (members) => {
  const s = new Set()
  for (const m of members) for (const p of m.host) s.add(p)
  for (const m of members) for (const p of m.client) s.add(p)
  return s.size
}

console.log(`distinct services touched          ${byService.size} (${split.host.length} Host, ${split.client.length} Client, ${byService.size - nonKernel.length} Cordis runtime)`)
console.log(`distinct service.member pairs      ${rows.length}`)
console.log(`declared as an injected service    ${injected.size}`)
for (const side of ['host', 'client']) {
  console.log('')
  console.log(`--- ${side.toUpperCase()}-side services ---`)
  console.log('service'.padEnd(22) + 'pkgs  members  declared  top members')
  for (const [service, members] of split[side]) {
    if (pkgCount(members) < 3) continue
    const top = members.slice(0, 6).map((m) => `${m.member}(${m.call + m.read})`).join(', ')
    console.log(`${service.padEnd(22)} ${String(pkgCount(members)).padStart(4)}  ${String(members.length).padStart(7)}  ${(injected.has(service) ? 'yes' : '  -').padStart(8)}  ${top}`)
  }
}

console.log('')
console.log('--- top 60 service.member pairs (all sides) ---')
for (const r of rows.slice(0, 60)) {
  console.log(`${(r.service + '.' + r.member).padEnd(46)} host=${String(r.host.size).padStart(3)} client=${String(r.client.size).padStart(3)}  call=${String(r.call).padStart(4)}  read=${String(r.read).padStart(4)}${r.destructured ? '  destructured=' + r.destructured : ''}`)
}

const out = path.join(HERE, 'service-surface.json')
fs.writeFileSync(out, JSON.stringify({
  services: byService.size,
  host: split.host.map(([s, m]) => s),
  client: split.client.map(([s, m]) => s),
  kernel: [...byService].filter(([s]) => KERNEL_SERVICES.has(s)).map(([s]) => s),
  rows: rows.map((r) => ({ ...r, host: [...r.host], client: [...r.client] })),
}, null, 1) + '\n')
console.log(`\nfull report: ${out}`)
