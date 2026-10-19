import * as React from 'react'
function Icon({ children }: { children?: React.ReactNode }) { return React.createElement('span', { className: 'icon' }, children) }
export function Tooltip({ children }: any) { return children }
export const IconLikeOutlineRegular = () => Icon({ children: '♥' })
export const IconLikeFillRegular = () => Icon({ children: '♥' })
export const IconDislikeOutlineRegular = () => Icon({ children: '♡' })
export const IconDislikeFillRegular = () => Icon({ children: '♡' })
export const IconChevronDownOutlineRegular = () => Icon({ children: '⌄' })

export function Toast({ text }: any) { return React.createElement('div', { className: 'toast' }, text) }
export function Modal({ open, title, closeLabel, onClose, footer, children }: any) {
  if (!open) return null
  return React.createElement('div', { className: 'modal-backdrop' },
    React.createElement('div', { className: 'modal' },
      React.createElement('div', { className: 'modal-head' }, title, React.createElement('button', { onClick: onClose }, closeLabel)),
      React.createElement('div', { className: 'modal-body' }, children),
      React.createElement('div', { className: 'modal-foot' }, footer)))
}
export function Button({ children, onClick, disabled }: any) { return React.createElement('button', { onClick, disabled }, children) }
export const IconWarningOutlineRegular = () => Icon({ children: '!' })
