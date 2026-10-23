import { createContext, useSyncExternalStore } from 'react'

/** On a phone, the sidebar is a drawer that each page's header opens (ADR 0044). */
export const Drawer = createContext<{ open: boolean; toggle: () => void } | null>(null)

const PHONE = '(max-width: 767px)'

/** A phone's width, where the shell folds into one column. */
export function phone(): boolean {
  return typeof window !== 'undefined' && Boolean(window.matchMedia?.(PHONE).matches)
}

/** Whether the window is a phone's width, following it as it changes. */
export function usePhone(): boolean {
  return useSyncExternalStore(
    (changed) => {
      const query = window.matchMedia?.(PHONE)
      query?.addEventListener('change', changed)
      return () => query?.removeEventListener('change', changed)
    },
    phone,
    () => false,
  )
}
