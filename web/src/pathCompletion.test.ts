import { describe, expect, it } from 'vitest'
import { completions, sharedStart, splitTyped, withoutTrailingSeparator } from './pathCompletion'

const folders = [
  { name: '.config', git: false },
  { name: 'Silt', git: true },
  { name: 'silt-old', git: false },
  { name: 'site', git: false },
]

describe('path completion', () => {
  it('completes the part after the last separator of either kind', () => {
    expect(splitTyped('app')).toBeNull()
    expect(splitTyped('/home/me/co')).toEqual({ dir: '/home/me/', partial: 'co' })
    expect(splitTyped('C:\\Users\\me\\')).toEqual({ dir: 'C:\\Users\\me\\', partial: '' })
  })

  it('matches names by their start in any case, hiding dot folders until a dot is typed', () => {
    expect(completions(folders, '').map((folder) => folder.name)).toEqual(['Silt', 'silt-old', 'site'])
    expect(completions(folders, 'sil').map((folder) => folder.name)).toEqual(['Silt', 'silt-old'])
    expect(completions(folders, '.').map((folder) => folder.name)).toEqual(['.config'])
  })

  it('extends to what every match shares', () => {
    expect(sharedStart(['Silt', 'silt-old'])).toBe('Silt')
    expect(sharedStart(['Silt', 'site'])).toBe('Si')
    expect(sharedStart([])).toBe('')
  })

  it('opens a completed folder without its trailing separator, but keeps a root', () => {
    expect(withoutTrailingSeparator('/home/me/app/')).toBe('/home/me/app')
    expect(withoutTrailingSeparator('/')).toBe('/')
    expect(withoutTrailingSeparator('C:\\')).toBe('C:\\')
  })
})
