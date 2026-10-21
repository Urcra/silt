import { createContext, type RefObject } from 'react'

/** The element the main panel scrolls in, for lists that render only what's in view. */
export const MainScroll = createContext<RefObject<HTMLElement | null>>({ current: null })
