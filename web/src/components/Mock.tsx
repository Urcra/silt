import type { ReactNode } from 'react'
import { type MockId, mocksOn } from '../mocks'

/** Marks what it wraps as mocked data (`data-mock`), and leaves it out when mocks are off. */
export function Mock({ id, children }: { id: MockId; children: ReactNode }) {
  if (!mocksOn) return null
  return (
    <span data-mock={id} className="contents">
      {children}
    </span>
  )
}
