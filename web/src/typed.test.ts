import { describe, expect, it } from 'vitest'
import { chartTable, csvOf, layout, parsed, partDocument, shown, slideFromHash } from './typed'

describe('the artifact types', () => {
  it('opens a deck at the slide a link names, within the deck', () => {
    expect(slideFromHash('#3', 5)).toBe(2)
    expect(slideFromHash('#9', 5)).toBe(4)
    expect(slideFromHash('#0', 5)).toBe(0)
    expect(slideFromHash('#artifact-x', 5)).toBe(0)
  })

  it('places boards in a row where they give no position', () => {
    const board = (id: string, width: number, x?: number) => ({ id, name: id, width, height: 10, html: '', x })
    const placed = layout([board('a', 100), board('b', 200), board('c', 50, 1000)])
    expect(placed.map((each) => each.x)).toEqual([0, 180, 1000])
  })

  it('turns a chart into a table and a table into CSV', () => {
    const table = chartTable([
      { name: 'p50', points: [['a', 1], ['b', 2]] },
      { name: 'p95', points: [['b', 5]] },
    ])
    expect(table).toEqual({ columns: ['x', 'p50', 'p95'], rows: [['a', 1, null], ['b', 2, 5]] })
    expect(csvOf(['name', 'note'], [['a, b', 'say "hi"']])).toBe('name,note\n"a, b","say ""hi"""')
    expect(shown(1234.56789)).toMatch(/1.?234\.568/)
  })

  it('renders a slide at its stage size in a sealed document', () => {
    const doc = partDocument('<h1>One</h1>', 'h1{color:red}', 1280, 720, null, 'dark')
    expect(doc).toContain('width:1280px;height:720px')
    expect(doc).toContain("setAttribute('data-theme','dark')")
    expect(doc).toContain('<h1>One</h1>')
    expect(parsed('{bad')).toBeNull()
  })
})
