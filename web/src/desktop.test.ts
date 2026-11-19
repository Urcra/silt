import { expect, test } from 'vitest'
import { hexOf, themeLink } from './desktop'

test('a computed colour becomes hex, and the link names the theme for the window', () => {
  expect(hexOf('rgb(30, 30, 46)')).toBe('#1e1e2e')
  expect(hexOf('rgba(246, 241, 232, 0.5)')).toBe('#f6f1e8')
  expect(hexOf('transparent')).toBeNull()
  expect(themeLink('dark', '#1e1e2e', '#cdd6f4')).toBe('silt://theme/?appearance=dark&bg=%231e1e2e&fg=%23cdd6f4')
})
