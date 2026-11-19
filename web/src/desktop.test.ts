import { expect, test } from 'vitest'
import { hexOf, machinesLink, themeLink } from './desktop'

test('a computed colour becomes hex, and the link names the theme for the window', () => {
  expect(hexOf('rgb(30, 30, 46)')).toBe('#1e1e2e')
  expect(hexOf('rgba(246, 241, 232, 0.5)')).toBe('#f6f1e8')
  expect(hexOf('transparent')).toBeNull()
  expect(themeLink('dark', '#1e1e2e', '#cdd6f4')).toBe('silt://theme/?appearance=dark&bg=%231e1e2e&fg=%23cdd6f4')
})

test('a machines link carries its fields and the window’s key, leaving out what’s empty', () => {
  expect(machinesLink('test', { name: 'home gpu', destination: 'me@vps.example.com', port: null, add: 1 }, 'k3y')).toBe(
    'silt://machines/test?name=home+gpu&destination=me%40vps.example.com&add=1&key=k3y',
  )
  expect(machinesLink('change', { name: 'vps', destination: 'gpu-box', port: 2222, was: 'old' }, 'k3y')).toBe(
    'silt://machines/change?name=vps&destination=gpu-box&port=2222&was=old&key=k3y',
  )
})
