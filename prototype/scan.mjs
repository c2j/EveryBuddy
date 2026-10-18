#!/usr/bin/env node
/**
 * Measures how tightly each dsh package couples to the runtime state that a
 * Rust kernel would own.
 *
 * Usage:  node scan.mjs [repoRoot]
 *
 * For every package it parses `src/**` with the TypeScript compiler API and
 * records, per package:
 *
 *   authorityValues  runtime (non-type-only) imports from authority packages
 *   kernelValues     runtime imports from the Cordis kernel modules
 *   identity         `instanceof` / class heritage / `new` on authority symbols
 *   waterfall        `.waterfall(` call sites
 *   sessionDirect    `ctx.session` / `ctx.sessions` member access
 *   inject           service names declared through `inject`
 *   client           whether the file lives in a Client face
 *
 * Output is a human summary plus `report.json` next to this script.
 *
 * The `B-viable` verdict consults `facade-safe.json` (module -> symbols the JS
 * shim can re-export with identical semantics). Absent that file, every
 * authority symbol counts as unsafe.
 */
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(process.argv[2] ?? process.cwd())
const require = createRequire(path.join(ROOT, 'package.json'))
const ts = require('typescript')

/** Packages whose runtime state a Rust kernel would own in route B. */
const AUTHORITY_DIRS =
  /^(core\/(agent|agent-loop|agent-default-model|agent-tool-presentation|session|system-prompt|tools)|llm\/llm|session\/(session|session-format|session-format-catalog|session-projection)|boot\/|api\/)/
/** The vendored Cordis runtime every plugin imports. */
const KERNEL_NAMES = new Set([
  '@deepseek-ai/cordis',
  '@deepseek-ai/cordis-plugin-loader',
  '@deepseek-ai/cordis-plugin-include',
  '@deepseek-ai/cordis-plugin-hmr',
  '@deepseek-ai/cordis-plugin-group',
  '@deepseek-ai/cordis-plugin-timer',
  '@deepseek-ai/cordis-plugin-logger-console',
])
/** Stateless helper libraries a JS shim may keep as-is. */
const HELPER_NAMES = new Set([
  '@deepseek-ai/schemastery',
  '@deepseek-ai/cosmokit',
  '@deepseek-ai/dsh-brand',
  '@deepseek-ai/dsh-scope',
  '@deepseek-ai/dsh-util-values',
])

const facadeSafe = fs.existsSync(path.join(HERE, 'facade-safe.json'))
  ? JSON.parse(fs.readFileSync(path.join(HERE, 'facade-safe.json'), 'utf8'))
  : {}

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
      const json = JSON.parse(fs.readFileSync(manifest, 'utf8'))
      out.push({
        id: `${group}/${dir}`,
        name: json.name,
        dir: pkgDir,
        authority: AUTHORITY_DIRS.test(`${group}/${dir}`),
      })
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

const isClientFile = (pkg, file) => {
  const rel = path.relative(pkg.dir, file)
  return pkg.id.startsWith('client/') || /(^|\/)client(\/|$)/.test(rel) || pkg.id.includes('-client-')
}

function classifyModule(spec, pkgById) {
  if (KERNEL_NAMES.has(spec)) return 'kernel'
  if (HELPER_NAMES.has(spec)) return 'helper'
  const owner = pkgById.get(spec)
  if (owner?.authority) return 'authority'
  return 'other'
}

/** Walks one file and returns every coupling signal it carries. */
function analyseFile(pkg, file, pkgById) {
  const text = fs.readFileSync(file, 'utf8')
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const res = {
    client: isClientFile(pkg, file),
    authorityValues: new Map(), // module -> Set(symbol)
    kernelValues: new Map(),
    helperValues: new Map(),
    identity: [], // { symbol, module, kind }
    waterfall: 0,
    sessionDirect: 0,
    inject: new Set(),
  }
  const bindings = new Map() // local name -> { module, symbol, kind }

  for (const stmt of sf.statements) {
    if (!ts.isImportDeclaration(stmt)) continue
    const spec = ts.isStringLiteral(stmt.moduleSpecifier) ? stmt.moduleSpecifier.text : ''
    const kind = classifyModule(spec, pkgById)
    if (kind === 'other' && !spec.startsWith('@deepseek-ai/dsh')) continue
    const clause = stmt.importClause
    if (!clause) continue
    const bucket = kind === 'kernel' ? res.kernelValues : kind === 'authority' ? res.authorityValues : kind === 'helper' ? res.helperValues : null
    const record = (symbol, local, typeOnly) => {
      if (typeOnly) return
      bindings.set(local, { module: spec, symbol })
      if (bucket) {
        if (!bucket.has(spec)) bucket.set(spec, new Set())
        bucket.get(spec).add(symbol)
      }
    }
    if (clause.name) record('default', clause.name.text, clause.isTypeOnly)
    const nb = clause.namedBindings
    if (nb && ts.isNamespaceImport(nb)) record('*', nb.name.text, clause.isTypeOnly)
    if (nb && ts.isNamedImports(nb)) {
      for (const el of nb.elements) {
        const typeOnly = clause.isTypeOnly || el.isTypeOnly
        record((el.propertyName ?? el.name).text, el.name.text, typeOnly)
      }
    }
  }

  const resolve = (node) => (ts.isIdentifier(node) ? bindings.get(node.text) : undefined)

  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      const callee = node.expression
      if (ts.isPropertyAccessExpression(callee) && callee.name.text === 'waterfall') res.waterfall++
      if (ts.isIdentifier(callee) && callee.text === 'waterfall') res.waterfall++
      if (ts.isPropertyAccessExpression(callee) && callee.expression.kind === ts.SyntaxKind.SuperKeyword) {
        const arg = node.arguments[1]
        if (arg && ts.isStringLiteral(arg)) res.inject.add(`provides:${arg.text}`)
      }
    }
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.InstanceOfKeyword) {
      const binding = resolve(node.right)
      if (binding) res.identity.push({ symbol: binding.symbol, module: binding.module, kind: 'instanceof' })
    }
    if (ts.isNewExpression(node)) {
      const binding = resolve(node.expression)
      if (binding) res.identity.push({ symbol: binding.symbol, module: binding.module, kind: 'new' })
    }
    if ((ts.isClassDeclaration(node) || ts.isClassExpression(node)) && node.heritageClauses) {
      for (const clause of node.heritageClauses) {
        for (const type of clause.types) {
          const binding = resolve(type.expression)
          if (binding) res.identity.push({ symbol: binding.symbol, module: binding.module, kind: 'extends' })
        }
      }
    }
    if (ts.isPropertyAccessExpression(node) && (node.name.text === 'session' || node.name.text === 'sessions')) {
      const base = node.expression
      if (ts.isIdentifier(base) && base.text === 'ctx') res.sessionDirect++
    }
    const collectInject = (n) => {
      if (ts.isStringLiteral(n)) res.inject.add(n.text)
      else if (ts.isArrayLiteralExpression(n)) n.elements.forEach(collectInject)
      else if (ts.isObjectLiteralExpression(n)) {
        for (const p of n.properties) {
          if (ts.isPropertyAssignment(p) && ['required', 'optional'].includes(p.name.getText(sf))) collectInject(p.initializer)
        }
      }
    }
    if (ts.isPropertyAssignment(node) && node.name.getText(sf) === 'inject') collectInject(node.initializer)
    if (ts.isPropertyDeclaration(node) && node.name.getText(sf) === 'inject' && node.initializer) collectInject(node.initializer)
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === 'inject' && node.initializer) {
      collectInject(node.initializer)
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return res
}

const packages = listPackages()
const pkgById = new Map(packages.map((p) => [p.name, p]))
const symbolUse = new Map() // module -> symbol -> { files, identity, packages:Set, kinds:Map }
const report = []

for (const pkg of packages) {
  const files = listSources(pkg.dir)
  if (files.length === 0) {
    report.push({ id: pkg.id, name: pkg.name, authority: pkg.authority, empty: true })
    continue
  }
  const agg = {
    authorityValues: new Map(),
    kernelValues: new Map(),
    helperValues: new Map(),
    identity: new Map(),
    waterfall: 0,
    sessionDirect: 0,
    inject: new Set(),
    hostFiles: 0,
    clientFiles: 0,
  }
  for (const file of files) {
    const r = analyseFile(pkg, file, pkgById)
    if (r.client) agg.clientFiles++
    else agg.hostFiles++
    for (const [mod, syms] of r.authorityValues) {
      if (!agg.authorityValues.has(mod)) agg.authorityValues.set(mod, new Set())
      for (const s of syms) {
        agg.authorityValues.get(mod).add(s)
        if (!symbolUse.has(mod)) symbolUse.set(mod, new Map())
        const rec = symbolUse.get(mod)
        if (!rec.has(s)) rec.set(s, { files: 0, identity: 0, packages: new Set(), kinds: new Map() })
        const e = rec.get(s)
        e.files++
        e.packages.add(pkg.id)
      }
    }
    for (const [mod, syms] of r.kernelValues) {
      if (!agg.kernelValues.has(mod)) agg.kernelValues.set(mod, new Set())
      for (const s of syms) agg.kernelValues.get(mod).add(s)
    }
    for (const [mod, syms] of r.helperValues) {
      if (!agg.helperValues.has(mod)) agg.helperValues.set(mod, new Set())
      for (const s of syms) agg.helperValues.get(mod).add(s)
    }
    for (const hit of r.identity) {
      const key = `${hit.module}:${hit.symbol}`
      agg.identity.set(key, (agg.identity.get(key) ?? 0) + 1)
      const rec = symbolUse.get(hit.module)?.get(hit.symbol)
      if (rec) {
        rec.identity++
        rec.kinds.set(hit.kind, (rec.kinds.get(hit.kind) ?? 0) + 1)
      }
    }
    agg.waterfall += r.waterfall
    agg.sessionDirect += r.sessionDirect
    for (const n of r.inject) agg.inject.add(n)
  }

  const authorityValueList = []
  let unsafeAuthority = []
  for (const [mod, syms] of agg.authorityValues) {
    for (const s of syms) {
      authorityValueList.push(`${mod.replace('@deepseek-ai/dsh-', '')}#${s}`)
      const safe = facadeSafe[mod]?.includes(s)
      if (!safe) unsafeAuthority.push(`${mod}#${s}`)
    }
  }
  authorityValueList.sort()
  unsafeAuthority.sort()
  const identityAuthority = [...agg.identity.keys()].filter((k) => k.startsWith('@deepseek-ai/dsh-'))
  const bViable = unsafeAuthority.length === 0 && identityAuthority.length === 0 && agg.waterfall === 0 && agg.sessionDirect === 0

  report.push({
    id: pkg.id,
    name: pkg.name,
    authority: pkg.authority,
    hostFiles: agg.hostFiles,
    clientFiles: agg.clientFiles,
    authorityValues: authorityValueList,
    unsafeAuthority,
    identityAuthority,
    kernelValues: [...agg.kernelValues].flatMap(([m, s]) => [...s].map((x) => `${m}#${x}`)).sort(),
    helperValues: [...agg.helperValues].flatMap(([m, s]) => [...s].map((x) => `${m}#${x}`)).sort(),
    waterfall: agg.waterfall,
    sessionDirect: agg.sessionDirect,
    inject: [...agg.inject].sort(),
    bViable,
  })
}

// ---- summary -------------------------------------------------------------

const active = report.filter((r) => !r.empty && r.hostFiles > 0)
const nonAuthority = active.filter((r) => !r.authority)
const tally = (pred) => nonAuthority.filter(pred).length

const buckets = {
  'no authority value import': tally((r) => r.authorityValues.length === 0),
  'authority values (facade-safe only)': tally((r) => r.authorityValues.length > 0 && r.unsafeAuthority.length === 0),
  'authority values (needs JS behavior)': tally((r) => r.unsafeAuthority.length > 0),
  'authority identity (instanceof/extends/new)': tally((r) => r.identityAuthority.length > 0),
  'uses waterfall': tally((r) => r.waterfall > 0),
  'touches ctx.session directly': tally((r) => r.sessionDirect > 0),
  'B-viable as-is': tally((r) => r.bViable),
}
const unionBreakers = nonAuthority.filter(
  (r) => r.unsafeAuthority.length > 0 || r.identityAuthority.length > 0 || r.waterfall > 0 || r.sessionDirect > 0,
).length

const totalPackages = packages.length
const withSrc = report.filter((r) => !r.empty).length
const authorityPackages = packages.filter((p) => p.authority).length
const totalSrcFiles = report.reduce((a, r) => a + (r.hostFiles ?? 0) + (r.clientFiles ?? 0), 0)

console.log(`repo                     ${ROOT}`)
console.log(`packages                 ${totalPackages} (${withSrc} with src, ${authorityPackages} authority, ${nonAuthority.length} plugin-side host packages)`)
console.log(`src files scanned        ${totalSrcFiles}`)
console.log('')
console.log('--- coupling buckets (plugin-side host packages) ---')
for (const [k, v] of Object.entries(buckets)) {
  console.log(`  ${k.padEnd(44)} ${String(v).padStart(3)}  (${((v / nonAuthority.length) * 100).toFixed(1)}%)`)
}
console.log(`  ${'breaks under a Rust kernel (any reason)'.padEnd(44)} ${String(unionBreakers).padStart(3)}  (${((unionBreakers / nonAuthority.length) * 100).toFixed(1)}%)`)
console.log('')
console.log('--- authority runtime symbols imported by plugins (module#symbol  valueFiles  identityFiles) ---')
for (const [mod, rec] of [...symbolUse].sort((a, b) => b[1].size - a[1].size)) {
  const rows = [...rec].sort((a, b) => b[1].files - a[1].files)
  console.log(`${mod}`)
  for (const [sym, e] of rows) {
    console.log(`    ${sym.padEnd(38)} files=${String(e.files).padStart(4)}  identity=${String(e.identity).padStart(3)}  pkgs=${e.packages.size}`)
  }
}
console.log('')
console.log('--- kernel runtime symbols imported by plugins ---')
const kernelUse = new Map()
for (const r of report) for (const k of r.kernelValues ?? []) {
  const m = k.split('#')
  if (!kernelUse.has(m[0])) kernelUse.set(m[0], new Set())
  kernelUse.get(m[0]).add(m[1])
}
for (const [mod, syms] of kernelUse) console.log(`  ${mod}: ${[...syms].sort().join(', ')}`)

const outPath = path.join(HERE, 'report.json')
fs.writeFileSync(outPath, JSON.stringify({ root: ROOT, buckets, unionBreakers, report }, null, 1) + '\n')
console.log(`\nfull report: ${outPath}`)
