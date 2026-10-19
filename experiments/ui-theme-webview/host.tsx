import * as React from 'react'
import * as jsxRuntime from 'react/jsx-runtime'
import { createRoot } from 'react-dom/client'
import * as primitives from '@deepseek-ai/dsh-client-ui-primitives'
import * as store from '@deepseek-ai/dsh-client-store'

const modules: Record<string, any> = {
  react: React,
  'react/jsx-runtime': jsxRuntime,
  '@deepseek-ai/dsh-client-ui-primitives': primitives,
  '@deepseek-ai/dsh-client-store': store,
}
const pending: any[] = []
;(window as any).__ModuleLoader__ = {
  load(entry: any) { pending.push(entry) },
}
const labels: Record<string, string> = {
  'appearance.title': 'Appearance',
  'appearance.light': 'Light',
  'appearance.dark': 'Dark',
  'appearance.system': 'System',
  'fontSize.text.title': 'Text size',
  'fontSize.text.description': 'Only affects conversation text',
  'fontSize.code.title': 'Code size',
  'fontSize.code.description': 'Code blocks, inline code and tool output',
  'fontSize.terminal.title': 'Terminal size',
  'fontSize.terminal.description': 'Sidebar terminal',
  'fontSize.unit': 'px',
}
function t(key: string) { return labels[key] ?? key }
function makeScope() {
  const value: any = { preference: 'system', fontSize: 14, codeFontSize: 11, terminalFontSize: 13, textFontFamily: '', codeFontFamily: '', terminalFontFamily: '' }
  const listeners = new Set<() => void>()
  return {
    subscribe(cb: () => void) { listeners.add(cb); return () => listeners.delete(cb) },
    getSnapshot() { return { value } },
    set(field: string, next: any) { value[field] = next; listeners.forEach((cb) => cb()); return Promise.resolve() },
    mutate(ops: any[]) { for (const op of ops) if (op.op === 'set') value[op.path[0]] = op.value; listeners.forEach((cb) => cb()); return Promise.resolve() },
  }
}

function run() {
  try {
  const handlers: Record<string, Function[]> = {}
  const ctx: any = {
    effect(fn: () => any) { return fn() },
    inject(_names: string[], cb: (scope: any) => void) { cb({ shortcuts: { runtime: 'web' }, locale: { bind: () => (key: string) => key } }) },
    locale: { register() {} },
    provide() {},
    on(name: string, cb: Function) { (handlers[name] ??= []).push(cb) },
    emit(name: string, ...args: any[]) {
      (handlers[name] ?? []).forEach((cb) => cb(...args))
      if (name === 'theme/change') {
        const snapshot = args[0]
        const dark = snapshot?.active?.colorScheme === 'dark'
        document.body.toggleAttribute('data-ds-dark-theme', dark)
        for (const [token, value] of Object.entries(snapshot?.active?.tokens ?? {})) {
          document.documentElement.style.setProperty(token, String(value))
        }
      }
    },
    shortcuts: { register() {} },
    configForms: { get() { return makeScope() } },
    slots: {
      inject(_name: string, cb: () => void) { cb() },
      register(def: any, Component: any) {
        const root = document.getElementById('root')!
        const host = document.createElement('div')
        host.className = 'plugin-row'
        root.appendChild(host)
        const props: any = { t, useStore: def.store?.useStore, ...(def.inject ? def.inject(def.store?.actions ?? {}) : {}) }
        createRoot(host).render(React.createElement(Component, props))
      },
    },
  }
  for (const entry of pending) {
    const mod = entry.factory((name: string) => {
      if (name in modules) return modules[name]
      throw new Error(`unknown shared module ${name}`)
    })
    mod.inject ?? []
    mod.apply(ctx)
  }
  } catch (error) {
    const root = document.getElementById('root')!
    root.textContent = 'HOST ERROR: ' + String(error)
    throw error
  }
}
;(window as any).__dshRun = run
