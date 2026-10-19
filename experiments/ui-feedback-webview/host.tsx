import * as React from 'react'
import * as jsxRuntime from 'react/jsx-runtime'
import { createRoot } from 'react-dom/client'
import * as primitives from '@deepseek-ai/dsh-client-ui-primitives'
import * as store from '@deepseek-ai/dsh-client-store'

const modules: Record<string, any> = { react: React, 'react/jsx-runtime': jsxRuntime, '@deepseek-ai/dsh-client-ui-primitives': primitives, '@deepseek-ai/dsh-client-store': store }
const pending: any[] = []
;(window as any).__ModuleLoader__ = { load(entry: any) { pending.push(entry) } }
const labels: Record<string, string> = {
  'action.like': 'Like', 'action.likeActive': 'Liked',
  'action.dislike': 'Dislike', 'action.dislikeActive': 'Disliked',
  'error.generic': 'Could not save feedback', 'error.load': 'Could not load feedback',
}
function t(key: string) { return labels[key] ?? key }
function makeStore() {
  let value: any = { preference: 'light' }
  const listeners = new Set<() => void>()
  return {
    subscribe(cb: () => void) { listeners.add(cb); return () => listeners.delete(cb) },
    getSnapshot() { return { value } },
    set(field: string, next: any) { value[field] = next; listeners.forEach((cb) => cb()); return Promise.resolve() },
    mutate(ops: any[]) { for (const op of ops) if (op.op === 'set') value[op.path[0]] = op.value; listeners.forEach((cb) => cb()); return Promise.resolve() },
  }
}
function useFeedback(feedback: any, selector: (view: any) => any) {
  return React.useSyncExternalStore(feedback.subscribe, () => selector(feedback.getSnapshot()), () => selector(feedback.getSnapshot()))
}
function Actions(props: any) {
  const feedback = props.hooks.feedback
  const view = useFeedback(feedback, (v: any) => v.items.get(props.messageId))
  return React.createElement(React.Fragment, null,
    React.createElement('button', { onClick: () => props.openDialog('positive') }, '♥ Like'),
    React.createElement('button', { onClick: () => props.openDialog('negative') }, '♡ Dislike'),
    React.createElement('span', { style: { marginLeft: 8, color: view?.rating ? '#0a7' : '#777' } }, view?.rating ?? 'no rating'),
  )
}
function run() {
  const status = document.getElementById('status')!
  const items = new Map<string, any>()
  const remote = {
    messageFeedback: {
      list: async () => ({ ok: true, value: { ok: true, value: { items: [...items.values()] } } }),
      put: async (args: any) => {
        const item = { messageId: args.messageId, rating: args.rating, version: (items.get(args.messageId)?.version ?? 0) + 1 }
        items.set(args.messageId, item)
        return { ok: true, value: { ok: true, value: item } }
      },
      delete: async (args: any) => { items.delete(args.messageId); return { ok: true, value: { ok: true } } },
    },
    sessionFeedback: { record: async () => ({ ok: true, value: { ok: true } }) },
  }
  const handlers: Record<string, Function[]> = {}
  const ctx: any = {
    effect(fn: () => any) { return fn() },
    inject(_names: string[], cb: (scope: any) => void) { cb({ commandUi: { decorate() {} }, effect() {} }) },
    locale: { register() {}, bind: () => (key: string) => t(key) },
    provide() {}, on(name: string, cb: Function) { (handlers[name] ??= []).push(cb) },
    emit(name: string, ...args: any[]) { (handlers[name] ?? []).forEach((cb) => cb(...args)) },
    shortcuts: { runtime: 'web' }, configForms: { get: makeStore }, remote,
    slots: {
      inject(_name: string, cb: () => void) { cb() },
      register(def: any, Component: any) {
        if (def.name !== 'conversation.chat.assistant-actions') return
        const props = def.inject('session-demo')
        props.messageId = 'm1'
        props.useFeedback = (selector: any) => useFeedback(props.hooks.feedback, selector)
        props.openDialog = (rating: string) => { status.textContent = `dialog opened: ${rating}` }
        props.t = t
        const host = document.createElement('div')
        host.className = 'actions'
        document.getElementById('root')!.appendChild(host)
        createRoot(host).render(React.createElement(Actions, props))
      },
    },
  }
  for (const entry of pending) {
    const mod = entry.factory((name: string) => { if (name in modules) return modules[name]; throw new Error(`unknown ${name}`) })
    mod.apply(ctx)
  }
}
;(window as any).__dshRun = run
