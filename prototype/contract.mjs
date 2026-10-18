#!/usr/bin/env node
/**
 * Contract draft for the Host-side service surface, plus the synchronous-call
 * verdict that decides whether a cross-process kernel is viable.
 *
 * Two measurements are merged per `service.member` pair:
 *
 *  1. Declaration — the providing class is located through `super(ctx, 'name')`,
 *     and the member's declaration is read: kind, parameters, return type, and
 *     whether it is declared `async` (or returns a thenable).
 *
 *  2. Call sites — every `ctx.<service>.<member>` use is classified by the
 *     context it sits in:
 *       awaited     inside `await` or a `.then()` chain
 *       asyncFn     enclosing function is async, result not awaited
 *       syncFn      enclosing function is not async
 *       fieldInit   property or parameter initializer
 *       topLevel    module scope, not awaited
 *
 *  A pair is `sync-hard` when it is NOT declared async AND at least one call
 *  site is in `syncFn`, `fieldInit`, or un-awaited `topLevel` — such a member
 *  must return its value without an await, so a kernel in another process would
 *  need a blocking round trip or a local mirror.
 *
 * Usage: node contract.mjs [repoRoot]
 */
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(process.argv[2] ?? path.join(HERE, '..'))
const require = createRequire(path.join(ROOT, 'package.json'))
const ts = require('typescript')

function listPackages() {
  const out = []
  const packagesDir = path.join(ROOT, 'packages')
  for (const group of fs.readdirSync(packagesDir)) {
    const groupDir = path.join(packagesDir, group)
    if (!fs.statSync(groupDir).isDirectory()) continue
    for (const dir of fs.readdirSync(groupDir)) {
      const pkgDir = path.join(groupDir, dir)
      if (!fs.existsSync(path.join(pkgDir, 'package.json'))) continue
      out.push({ id: `${group}/${dir}`, dir: pkgDir })
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

const packages = listPackages()
const sourcesByPackage = new Map(packages.map((p) => [p.id, listSources(p.dir)]))

const parse = (file) => {
  const text = fs.readFileSync(file, 'utf8')
  return ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
}

const hasModifier = (node, kind) => !!node.modifiers?.some((m) => m.kind === kind)

// ---------------------------------------------------------------- providers

/**
 * A service can be declared in more than one place: an abstract `Service`
 * subclass states the base members, a concrete subclass adds the rest, and
 * some facilities are published with `ctx.provide('name', value)` instead of
 * `super(ctx, 'name')`. All three are collected so member lookup can search
 * every declaration.
 *
 * service name -> [{ pkgId, file, className, classNode, classes }]
 */
const providers = new Map()
const addProvider = (name, entry) => {
  if (!providers.has(name)) providers.set(name, [])
  const list = providers.get(name)
  if (list.some((e) => e.classNode === entry.classNode && e.pkgId === entry.pkgId)) return
  list.push(entry)
}

for (const pkg of packages) {
  const classes = new Map() // className -> ClassDeclaration, across the package
  const parsed = sourcesByPackage.get(pkg.id).map((f) => [f, parse(f)])
  for (const [, sf] of parsed) {
    const collect = (node) => {
      if ((ts.isClassDeclaration(node) || ts.isClassExpression(node)) && node.name) classes.set(node.name.text, node)
      ts.forEachChild(node, collect)
    }
    collect(sf)
  }
  for (const [file, sf] of parsed) {
    const enclosingClass = (node) => {
      let n = node
      while (n && !ts.isClassDeclaration(n) && !ts.isClassExpression(n)) n = n.parent
      return n
    }
    const scan = (node) => {
      if (ts.isClassDeclaration(node) && node.name) {
        for (const member of node.members) {
          if (!ts.isConstructorDeclaration(member)) continue
          const call = member.body && member.body.statements.find((s) => ts.isExpressionStatement(s) && ts.isCallExpression(s.expression) && s.expression.expression.kind === ts.SyntaxKind.SuperKeyword)
          const superCall = call && call.expression
          if (!superCall || !superCall.arguments) continue
          const nameArg = superCall.arguments.find((a) => ts.isStringLiteral(a) && !/^[./]/.test(a.text))
          if (!nameArg) continue
          addProvider(nameArg.text, { pkgId: pkg.id, file, className: node.name.text, classNode: node, classes, via: 'service' })
        }
      }
      // `ctx.provide('name', value)` publishes a facility without a Service subclass.
      if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'provide') {
        const nameArg = node.arguments[0]
        const valueArg = node.arguments[1]
        const owner = enclosingClass(node)
        if (nameArg && ts.isStringLiteral(nameArg) && owner && owner.name) {
          const isSelf = valueArg && valueArg.kind === ts.SyntaxKind.ThisKeyword
          if (!isSelf) return ts.forEachChild(node, scan)
          addProvider(nameArg.text, { pkgId: pkg.id, file, className: owner.name.text, classNode: owner, classes, via: 'provide' })
        }
      }
      ts.forEachChild(node, scan)
    }
    scan(sf)
  }
}

/**
 * Finds every declaration of a member across all classes that declare the
 * service. A `Service` subclass is the owning declaration; a facility
 * published through `ctx.provide` is a facade over it, so owning declarations
 * are searched first. Several declarations are reported when the same service
 * name is published on both the host and client planes.
 */
function findMembers(entries, member) {
  const ordered = [...entries].sort((a, b) => (a.via === b.via ? 0 : a.via === 'service' ? -1 : 1))
  const found = []
  for (const entry of ordered) {
    const decl = findMemberIn(entry, member)
    if (decl) found.push(decl)
  }
  return found
}

function findMemberIn(entry, member) {
  let node = entry.classNode
  const seen = new Set()
  while (node && !seen.has(node)) {
    seen.add(node)
    for (const m of node.members) {
      if (ts.isConstructorDeclaration(m)) continue
      const name = m.name && ts.isIdentifier(m.name) ? m.name.text : m.name && ts.isStringLiteral(m.name) ? m.name.text : undefined
      if (name !== member) continue
      const kind = ts.isMethodDeclaration(m) ? 'method' : ts.isGetAccessor(m) ? 'getter' : ts.isPropertyDeclaration(m) ? 'property' : ts.isSetAccessor(m) ? 'setter' : 'other'
      const typeText = m.type ? m.type.getText(node.getSourceFile()) : undefined
      const asyncDeclared = hasModifier(m, ts.SyntaxKind.AsyncKeyword) || (!!typeText && /^(Promise|Thenable|AsyncIterable)/.test(typeText))
      const params = ts.isMethodDeclaration(m)
        ? m.parameters.map((p) => `${p.name.getText(node.getSourceFile())}${p.type ? ': ' + p.type.getText(node.getSourceFile()) : ''}${p.questionToken ? '?' : ''}`).join(', ')
        : undefined
      return { kind, typeText, asyncDeclared, params, pkgId: entry.pkgId, via: entry.via, file: entry.file, className: entry.className }
    }
    const heritage = node.heritageClauses?.find((c) => c.token === ts.SyntaxKind.ExtendsKeyword)
    const base = heritage?.types[0]?.expression
    const baseName = base && ts.isIdentifier(base) ? base.text : undefined
    node = baseName ? entry.classes.get(baseName) : undefined
  }
  return undefined
}

// --------------------------------------------------------------- call sites

const stats = new Map()
const bump = (pair, bucket) => {
  if (!stats.has(pair)) stats.set(pair, { awaited: 0, asyncFn: 0, syncFn: 0, fieldInit: 0, topLevel: 0 })
  stats.get(pair)[bucket]++
}

function enclosingContext(node) {
  let n = node.parent
  while (n) {
    if (ts.isFunctionLike(n)) return hasModifier(n, ts.SyntaxKind.AsyncKeyword) ? 'asyncFn' : 'syncFn'
    if (ts.isPropertyDeclaration(n) || ts.isParameter(n) || ts.isPropertyAssignment(n)) return 'fieldInit'
    if (ts.isSourceFile(n)) return 'topLevel'
    n = n.parent
  }
  return 'topLevel'
}

function isAwaited(node) {
  let n = node
  while (n.parent) {
    if (ts.isAwaitExpression(n.parent)) return true
    if (ts.isPropertyAccessExpression(n.parent) && ['then', 'catch', 'finally'].includes(n.parent.name.text)) return true
    if (ts.isStatement(n.parent) || ts.isSourceFile(n.parent)) return false
    n = n.parent
  }
  return false
}

const isCtxName = (name) => name === 'ctx' || /Ctx$/.test(name)
const ctxBase = (node) => {
  if (ts.isIdentifier(node) && isCtxName(node.text)) return true
  if (ts.isPropertyAccessExpression(node) && node.name.text === 'ctx') return true
  return false
}
const stringArg = (n) => (n && ts.isStringLiteral(n) ? n.text : undefined)

for (const pkg of packages) {
  if (/^(core\/|llm\/|session\/|boot\/|api\/)/.test(pkg.id)) continue
  const files = sourcesByPackage.get(pkg.id)
  if (files.length === 0) continue
  for (const file of files) {
    const sf = parse(file)
    const alias = new Map()
    const visitAlias = (node) => {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
        const init = node.initializer
        if (ts.isPropertyAccessExpression(init) && ctxBase(init.expression)) alias.set(node.name.text, init.name.text)
        else if (ts.isCallExpression(init) && ts.isPropertyAccessExpression(init.expression) && init.expression.name.text === 'get' && ctxBase(init.expression.expression)) {
          const svc = stringArg(init.arguments[0])
          if (svc) alias.set(node.name.text, svc)
        }
      }
      ts.forEachChild(node, visitAlias)
    }
    visitAlias(sf)

    const visit = (node) => {
      let pair
      if (ts.isPropertyAccessExpression(node)) {
        const owner = node.expression
        if (ts.isPropertyAccessExpression(owner) && ctxBase(owner.expression)) pair = `${owner.name.text}.${node.name.text}`
        else if (ts.isIdentifier(owner) && alias.has(owner.text)) pair = `${alias.get(owner.text)}.${node.name.text}`
        else if (ts.isCallExpression(owner) && ts.isPropertyAccessExpression(owner.expression) && owner.expression.name.text === 'get' && ctxBase(owner.expression.expression)) {
          const svc = stringArg(owner.arguments[0])
          if (svc) pair = `${svc}.${node.name.text}`
        }
      }
      if (pair) bump(pair, isAwaited(node) ? 'awaited' : enclosingContext(node))
      ts.forEachChild(node, visit)
    }
    visit(sf)
  }
}

// ------------------------------------------------------------------ output

const surface = JSON.parse(fs.readFileSync(path.join(HERE, 'service-surface.json'), 'utf8'))
const hostPairs = surface.rows
  .filter((r) => r.host.length > 0)
  .map((r) => `${r.service}.${r.member}`)
const KERNEL = new Set(['logger', 'fiber', 'events', 'reflect', 'registry'])

const rows = []
for (const pair of hostPairs) {
  const dot = pair.indexOf('.')
  const service = pair.slice(0, dot)
  const member = pair.slice(dot + 1)
  const entries = providers.get(service)
  const decls = entries ? findMembers(entries, member) : []
  const decl = decls[0]
  const calls = stats.get(pair) ?? { awaited: 0, asyncFn: 0, syncFn: 0, fieldInit: 0, topLevel: 0 }
  const syncSites = calls.syncFn + calls.fieldInit + calls.topLevel
  // A member must answer without an await when its owning declaration is not
  // async and at least one call site sits outside an await.
  const asyncDeclared = decl ? decl.asyncDeclared : null
  const mixedDeclarations = decls.length > 1 && decls.some((d) => d.asyncDeclared !== decl.asyncDeclared)
  const syncHard = asyncDeclared !== true && syncSites > 0
  rows.push({
    pair, service, member,
    provider: decl ? decl.pkgId : entries ? entries[0].pkgId : null,
    declarations: decls.length,
    mixedDeclarations,
    className: decl?.className ?? null,
    kind: decl?.kind ?? null,
    params: decl?.params ?? null,
    type: decl?.typeText ?? null,
    asyncDeclared,
    calls,
    syncSites,
    syncHard,
    kernel: KERNEL.has(service),
  })
}

const covered = (n) => {
  const sorted = [...rows].sort((a, b) => (b.calls.awaited + b.calls.asyncFn + b.calls.syncFn + b.calls.fieldInit + b.calls.topLevel) - (a.calls.awaited + a.calls.asyncFn + a.calls.syncFn + a.calls.fieldInit + a.calls.topLevel))
  return sorted.slice(0, n)
}

for (const n of [33, 135, rows.length]) {
  const top = covered(n)
  const hard = top.filter((r) => r.syncHard).length
  const unknown = top.filter((r) => r.asyncDeclared === null).length
  console.log(`${String(n).padStart(4)} pairs -> sync-hard ${String(hard).padStart(3)} (${((hard / n) * 100).toFixed(1)}%)  provider-not-found ${unknown}`)
}

console.log('\n--- top 40 Host pairs: contract draft + sync verdict ---')
console.log('pair'.padEnd(40) + 'async?  syncSites  awaited  provider            signature')
for (const r of covered(40)) {
  const sig = r.kind === 'method' ? `${r.member}(${r.params ?? ''})${r.type ? ': ' + r.type : ''}`
    : r.kind === 'getter' ? `get ${r.member}(): ${r.type ?? '?'}`
    : r.kind === 'property' ? `${r.member}: ${r.type ?? '?'}`
    : (r.kind ?? 'NOT FOUND')
  console.log(
    r.pair.padEnd(40) +
    String(r.asyncDeclared === null ? '?' : r.asyncDeclared ? 'yes' : 'no').padEnd(8) +
    String(r.syncSites).padStart(6) + '  ' + String(r.calls.awaited).padStart(7) + '  ' +
    (r.provider ?? '-').padEnd(20) + sig.slice(0, 60),
  )
}

const hard = rows.filter((r) => r.syncHard)
console.log(`\nsync-hard pairs (must return without await): ${hard.length} of ${rows.length}`)
for (const r of hard.slice(0, 40)) {
  console.log(`  ${r.pair.padEnd(38)} sync=${r.syncSites} awaited=${r.calls.awaited}  ${r.provider ?? '-'}`)
}

const out = path.join(HERE, 'contract.json')
fs.writeFileSync(out, JSON.stringify(rows, null, 1) + '\n')
console.log(`\nfull report: ${out}`)
