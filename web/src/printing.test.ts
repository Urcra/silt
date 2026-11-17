import { describe, expect, it } from 'vitest'
import { printable } from './printing'

describe('a page printed in the browser', () => {
  it('goes under the artifact policy, in its head or at its start', () => {
    expect(printable('<html><head><title>t</title></head><body>x</body></html>')).toMatch(
      /^<html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none';/,
    )
    expect(printable('<p>x</p>')).toMatch(/^<meta http-equiv="Content-Security-Policy"/)
  })
})
