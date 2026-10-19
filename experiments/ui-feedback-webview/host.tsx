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
  'dialog.title': 'Send feedback', 'close': 'Close', 'dialog.hint': 'Tell us more (optional)',
  'dialog.detail': 'Details', 'dialog.categories': 'What is this about?', 'submit': 'Submit',
  'submitting': 'Submitting…', 'toast.recorded': 'Feedback recorded',
  'category.task-result': 'Task result', 'category.instruction-following': 'Instruction following',
  'category.product-interaction': 'Product interaction', 'category.service-stability': 'Service stability',
  'category.resource-cost': 'Resource cost', 'category.security-privacy-permission': 'Security / privacy',
  'category.other': 'Other',
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
class Boundary extends React.Component<any, any> {
  state = { error: null as any }
  static getDerivedStateFromError(error: any) { return { error } }
  render() {
    if (this.state.error) return React.createElement('pre', { style: { color: '#b00', whiteSpace: 'pre-wrap' } }, String(this.state.error))
    return this.props.children
  }
}
function Actions(props: any) {
  const feedback = props.hooks.feedback
  const view = useFeedback(feedback, (v: any) => v.items.get(props.messageId))
  return React.createElement(React.Fragment, null,
    React.createElement('button', { onClick: () => props.openDialog(props.messageId, 'positive') }, '♥ Like'),
    React.createElement('button', { onClick: () => props.openDialog(props.messageId, 'negative') }, '♡ Dislike'),
    React.createElement('span', { style: { marginLeft: 8, color: view?.rating ? '#0a7' : '#777' } }, view?.rating ?? 'no rating'),
  )
}
function Overlay(props: any) {
  const state = React.useSyncExternalStore(props.hooks.dialog.subscribe, () => props.hooks.dialog.getSnapshot(), () => props.hooks.dialog.getSnapshot())
  return React.createElement('div', { style: { marginTop: 20 } },
    React.createElement('div', { style: { color: '#777', fontSize: 12 } }, `overlay target: ${state.target ? JSON.stringify(state.target) : 'closed'}`),
    React.createElement(props.Component, {
      useDialog: (selector: any) => selector(state),
      edit: props.edit, submit: props.submit, dismiss: props.dismiss,
      dismissFailure: props.dismissFailure, dismissToast: props.dismissToast, t: props.t,
    }))
}
function run() {
  // Real dsh snapshot fixture: snapshots/session/empty-response-retry-current
  const fixture = { sessionId: 'snapshot-empty-response-retry-current', messageId: 'snapshot-message-4', text: 'Recovered.' }
  const storageKey = `evobuddy-feedback:${fixture.sessionId}`
  const loadItems = () => {
    try {
      return new Map<string, any>(JSON.parse(localStorage.getItem(storageKey) ?? '[]'))
    } catch {
      return new Map<string, any>()
    }
  }
  const items = loadItems()
  const saveItems = () => {
    try {
      localStorage.setItem(storageKey, JSON.stringify([...items.entries()]))
    } catch {}
  }
  const remote = {
    messageFeedback: {
      list: async () => ({ ok: true, value: { ok: true, value: { items: [...items.values()] } } }),
      put: async (args: any) => {
        const item = { messageId: args.messageId, rating: args.rating, version: (items.get(args.messageId)?.version ?? 0) + 1 }
        items.set(args.messageId, item)
        saveItems()
        return { ok: true, value: { ok: true, value: item } }
      },
      delete: async (args: any) => { items.delete(args.messageId); saveItems(); return { ok: true, value: { ok: true } } },
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
      inject(name: string, cb: () => void) { (window as any).__slots = ((window as any).__slots ?? []).concat(name); cb() },
      register(def: any, Component: any) {
        const props = def.inject('session-demo')
        if (def.name === 'conversation.chat.assistant-actions') {
          props.messageId = fixture.messageId
          props.useFeedback = (selector: any) => useFeedback(props.hooks.feedback, selector)
          const open = props.openDialog
          props.openDialog = (...args: any[]) => {
            const status = document.getElementById('status')
            if (status) status.textContent = `openDialog(${args.join(', ')})`
            return open(...args)
          }
          props.t = t
          const host = document.createElement('div')
          host.className = 'actions'
          document.getElementById('root')!.appendChild(host)
          createRoot(host).render(React.createElement(Actions, props))
        } else if (def.name === 'conversation.input.overlay') {
          props.Component = Component
          props.t = t
          const host = document.getElementById('overlay')!
          host.textContent = 'overlay slot registered'
          createRoot(host).render(React.createElement(Boundary, null, React.createElement(Overlay, props)))
        }
      },
    },
  }
  ;(window as any).__slots = []
  for (const entry of pending) {
    const mod = entry.factory((name: string) => { if (name in modules) return modules[name]; throw new Error(`unknown ${name}`) })
    mod.apply(ctx)
  }
  const slots = document.getElementById('slots')
  if (slots) slots.textContent = 'slots injected: ' + ((window as any).__slots ?? []).join(' | ')
}
;(window as any).__dshRun = run
