import * as React from 'react'
function Icon({ children }: { children?: React.ReactNode }) { return React.createElement('span', { className: 'icon' }, children) }
export function Tooltip({ children }: any) { return children }
export const IconLikeOutlineRegular = () => Icon({ children: '♥' })
export const IconLikeFillRegular = () => Icon({ children: '♥' })
export const IconDislikeOutlineRegular = () => Icon({ children: '♡' })
export const IconDislikeFillRegular = () => Icon({ children: '♡' })
export const IconChevronDownOutlineRegular = () => Icon({ children: '⌄' })
