import type { ReactNode } from 'react'
import { type MockId, mocksOn } from '../mocks'

/**
 * Marks what it wraps as mocked data (`data-mock`), and leaves it out when mocks are off.
 * Wrap a value's label with it, so a build without mocks has no label left over.
 */
export function Mock({ id, children }: { id: MockId; children: ReactNode }) {
  if (!mocksOn) return null
  return (
    <span data-mock={id} className="contents">
      {children}
    </span>
  )
}
