import { afterEach, expect, test } from 'vitest'
import { endPreview, injectThemeCss, previewTheme, startTheme } from './theme'

afterEach(() => {
  document.head.innerHTML = ''
  delete document.documentElement.dataset.theme
  localStorage.clear()
})

function hostedPage() {
  const meta = document.createElement('meta')
  meta.name = 'silt-served-by'
  meta.content = 'server'
  document.head.appendChild(meta)
}

test('the theme goes in one style element, replaced rather than added to', () => {
  injectThemeCss(':root{--bg:#000000;}')
  injectThemeCss(':root{--bg:#ffffff;}')
  const styles = document.querySelectorAll('style#silt-theme')
  expect(styles).toHaveLength(1)
  expect(styles[0].textContent).toBe(':root{--bg:#ffffff;}')
})

test('a preview shows its own appearance and ends where it started', () => {
  document.documentElement.dataset.theme = 'light'
  previewTheme(':root{--bg:#1e1e2e;}', 'dark')
  previewTheme(':root{--bg:#282828;}', 'dark')
  expect(document.documentElement.dataset.theme).toBe('dark')
  expect(document.getElementById('silt-theme-preview')?.textContent).toBe(':root{--bg:#282828;}')
  endPreview()
  expect(document.documentElement.dataset.theme).toBe('light')
  expect(document.getElementById('silt-theme-preview')).toBeNull()

  delete document.documentElement.dataset.theme
  previewTheme(':root{}', 'light')
  injectThemeCss(':root{--bg:#eff1f5;}')
  expect(document.documentElement.dataset.theme).toBeUndefined()
  expect(document.getElementById('silt-theme-preview')).toBeNull()
})

test("the hosted app starts in the theme this browser last showed; a daemon's page keeps what it was served", () => {
  hostedPage()
  injectThemeCss(':root{--bg:#191724;}')
  localStorage.setItem('silt-theme', 'dark')
  document.getElementById('silt-theme')?.remove()
  startTheme()
  expect(document.getElementById('silt-theme')?.textContent).toBe(':root{--bg:#191724;}')
  expect(document.documentElement.dataset.theme).toBe('dark')

  document.head.innerHTML = '<style id="silt-theme">:root{--bg:#f6f1e8;}</style>'
  document.documentElement.dataset.theme = 'light'
  startTheme()
  expect(document.getElementById('silt-theme')?.textContent).toBe(':root{--bg:#f6f1e8;}')
  expect(document.documentElement.dataset.theme).toBe('light')
})
