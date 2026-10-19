import * as React from 'react'
export function createSnapshotStore(initial: any) {
  let value = initial
  const listeners = new Set<() => void>()
  return {
    getSnapshot: () => value,
    subscribe: (cb: () => void) => { listeners.add(cb); return () => listeners.delete(cb) },
    set: (next: any) => { value = next; listeners.forEach((cb) => cb()) },
  }
}
export function defineStore({ init, actions }: any) {
  let state = init()
  const listeners = new Set<() => void>()
  const api: any = {
    useStore(selector: (value: any) => any) {
      return React.useSyncExternalStore(
        (cb) => { listeners.add(cb); return () => listeners.delete(cb) },
        () => selector(state), () => selector(state))
    },
    actions: {},
  }
  for (const [name, fn] of Object.entries(actions ?? {})) {
    api.actions[name] = (...args: any[]) => { (fn as any)(state, ...args); listeners.forEach((cb) => cb()) }
  }
  return api
}
