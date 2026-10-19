import * as React from 'react'
function Icon({ children }: { children?: React.ReactNode }) {
  return React.createElement('span', { className: 'icon' }, children)
}
export const IconChevronDownOutlineRegular = () => Icon({ children: '⌄' })
export const IconChevronUpOutlineRegular = () => Icon({ children: '⌃' })
export const IconDarkOutlineMedium = () => Icon({ children: '◐' })
export const IconFollowsystemOutlineMedium = () => Icon({ children: '◑' })
export const IconLightOutlineMedium = () => Icon({ children: '☀' })
