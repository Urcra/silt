import { afterEach, expect, test, vi } from 'vitest'
import { draggingFileManagerFiles, fileManagerFiles, hexOf, localFiles, localFilesLink, machinesLink, themeLink } from './desktop'

test('a computed colour becomes hex, and the link names the theme for the window', () => {
  expect(hexOf('rgb(30, 30, 46)')).toBe('#1e1e2e')
  expect(hexOf('rgba(246, 241, 232, 0.5)')).toBe('#f6f1e8')
  expect(hexOf('transparent')).toBeNull()
  expect(themeLink('dark', '#1e1e2e', '#cdd6f4')).toBe('silt://theme/?appearance=dark&bg=%231e1e2e&fg=%23cdd6f4')
  const tones = ['#a62a2f', '#47701b', '#a87a0a', '#2b5f86', '#1e6b48']
  expect(themeLink('light', '#f2f5f0', '#1d2a22', { tones, show: 'auto' })).toBe(
    'silt://theme/?appearance=light&bg=%23f2f5f0&fg=%231d2a22&tones=%23a62a2f%2C%2347701b%2C%23a87a0a%2C%232b5f86%2C%231e6b48&seasonal=auto',
  )
})

test('a machines link carries its fields and the window’s key, leaving out what’s empty', () => {
  expect(machinesLink('test', { name: 'home gpu', destination: 'me@vps.example.com', port: null, add: 1 }, 'k3y')).toBe(
    'silt://machines/test?name=home+gpu&destination=me%40vps.example.com&add=1&key=k3y',
  )
  expect(machinesLink('change', { name: 'vps', destination: 'gpu-box', port: 2222, was: 'old' }, 'k3y')).toBe(
    'silt://machines/change?name=vps&destination=gpu-box&port=2222&was=old&key=k3y',
  )
})

/** A paste's or a drop's data as WebKitGTK gives it. */
function transfer(types: string[], data: Record<string, string> = {}, files: File[] = []): DataTransfer {
  return { types, files, getData: (type: string) => data[type] ?? '' } as unknown as DataTransfer
}

afterEach(() => {
  window.__siltDesktop = undefined
  vi.unstubAllGlobals()
})

test('files come from a file manager only in a desktop app that hands them over, and not for a link', () => {
  const copied = transfer(['text/uri-list'])
  expect(fileManagerFiles(copied)).toBe(false)
  expect(draggingFileManagerFiles(copied)).toBe(false)
  window.__siltDesktop = { key: 'k', localFiles: true }
  expect(fileManagerFiles(copied)).toBe(true)
  expect(draggingFileManagerFiles(copied)).toBe(true)
  expect(fileManagerFiles(transfer(['text/uri-list'], { 'text/uri-list': 'file:///home/me/budget.xls\r\n' }))).toBe(true)
  const link = transfer(['text/uri-list', 'text/plain'], { 'text/uri-list': 'https://example.com/' })
  expect(fileManagerFiles(link)).toBe(false)
  expect(draggingFileManagerFiles(link)).toBe(false)
  expect(fileManagerFiles(transfer(['text/plain'], { 'text/plain': 'words' }))).toBe(false)
  expect(fileManagerFiles(transfer(['Files', 'text/uri-list'], {}, [new File(['x'], 'shot.png')]))).toBe(false)
})

test('the window’s answer becomes files, fetched from where it serves them', async () => {
  window.__siltDesktop = { key: 'k3y', localFiles: true }
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => new Response(url.endsWith('/a') ? 'cells' : 'gone', { status: url.endsWith('/a') ? 200 : 404 })),
  )
  const told: string[] = []
  const answered = localFiles('pasted', (link) => {
    told.push(link)
    const request = Number(new URL(link).searchParams.get('request'))
    const detail = {
      request,
      files: [{ url: 'silt-files://localhost/a', name: 'budget.xls', size: 5, mime: 'application/vnd.ms-excel' }],
      refused: 'folder is a folder: attach the files in it',
    }
    window.dispatchEvent(new CustomEvent('silt-local-files', { detail: { ...detail, request: request + 1 } }))
    window.dispatchEvent(new CustomEvent('silt-local-files', { detail }))
  })
  const { files, refused } = await answered
  expect(told).toHaveLength(1)
  expect(told[0]).toMatch(/^silt:\/\/files\/pasted\?request=\d+&key=k3y$/)
  expect(files.map((file) => [file.name, file.type, file.size])).toEqual([['budget.xls', 'application/vnd.ms-excel', 5]])
  expect(refused).toBe('folder is a folder: attach the files in it')

  const missing = localFiles('dropped', (link) => {
    const request = Number(new URL(link).searchParams.get('request'))
    const files = [{ url: 'silt-files://localhost/b', name: 'later.txt', size: 4, mime: 'text/plain' }]
    window.dispatchEvent(new CustomEvent('silt-local-files', { detail: { request, files, refused: null } }))
  })
  await expect(missing).rejects.toThrow('later.txt couldn’t be read (404)')
})

test('a window that never answers is given up on', async () => {
  await expect(localFiles('dropped', () => {}, 10)).rejects.toThrow('The desktop app didn’t hand over the files dropped')
  expect(localFilesLink('pasted', 4, 'k')).toBe('silt://files/pasted?request=4&key=k')
})
