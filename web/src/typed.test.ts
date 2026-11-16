import { describe, expect, it } from 'vitest'
import { canvasOf, chartColors, chartTable, columnName, csvOf, dataPageOf, deckOf, layout, parsed, partDocument, rebaseDraft, shown, slideFromHash, splice } from './typed'

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

describe('chart colours', () => {
  it("come from the version's design system for the theme shown, or not at all", () => {
    const css = `:root, :root[data-theme="light"] {\n  --accent: #123456;\n  --chart-1: var(--accent);\n  --chart-2: #222222;\n  --chart-3: #333333;\n  --chart-4: #444444;\n  --chart-5: #555555;\n  --chart-6: #666666;\n}\n@media (prefers-color-scheme: dark) {\n  :root:not([data-theme="light"]) {\n    --chart-1: #aaaaaa;\n  }\n}\n:root[data-theme="dark"] {\n  --chart-1: #aaaaaa;\n}`
    expect(chartColors(css, 'light')).toEqual(['#123456', '#222222', '#333333', '#444444', '#555555', '#666666'])
    expect(chartColors(css, 'dark')).toBeNull()
    expect(chartColors(null, 'light')).toBeNull()
  })
})

describe("a peer's typed version", () => {
  it('is drawn only once it has the shape its viewer draws', () => {
    expect(deckOf('{"slides":[{"title":"One","html":"<p>1</p>"}]}')?.slides.length).toBe(1)
    expect(deckOf('{"slides":[{"title":1,"html":"x"}]}')).toBeNull()
    expect(deckOf('{"slides":"nope"}')).toBeNull()
    expect(deckOf(JSON.stringify({ slides: Array(501).fill({ title: 't', html: 'h' }) }))).toBeNull()
    expect(canvasOf('{"boards":[{"id":"a","name":"A","width":10,"height":10,"html":""}]}')?.boards.length).toBe(1)
    expect(canvasOf('{"boards":[{"id":"a","name":"A","width":-1,"height":10,"html":""}]}')).toBeNull()
    const page = { title: 'T', sources: [{ id: 's', name: 'S', kind: 'manual', detail: '', at: 'now' }], blocks: [] as unknown[] }
    expect(dataPageOf(JSON.stringify(page))?.title).toBe('T')
    page.blocks = [{ type: 'chart', chart: 'line', series: [{ name: 'a', points: [['x', 1], ['y', null]] }], source: 's' }]
    expect(dataPageOf(JSON.stringify(page))).not.toBeNull()
    page.blocks = [{ type: 'chart', chart: 'line', series: [{ name: 'a', points: [['x', 'one']] }], source: 's' }]
    expect(dataPageOf(JSON.stringify(page))).toBeNull()
    page.blocks = [{ type: 'table', columns: ['a'], rows: 'all of them', source: 's' }]
    expect(dataPageOf(JSON.stringify(page))).toBeNull()
  })
})

describe('docs and sheets', () => {
  it("sends one splice for what was typed, counting characters as the doc's text does", () => {
    expect(splice('# Plan\nship', '# Plan\n- ship')).toEqual({ pos: 7, delete: 0, insert: '- ' })
    expect(splice('abcdef', 'abXYef')).toEqual({ pos: 2, delete: 2, insert: 'XY' })
    expect(splice('é😀a', 'é😀')).toEqual({ pos: 2, delete: 1, insert: '' })
    expect(splice('same', 'same')).toBeNull()
  })
  it('moves what was typed during a send onto the text with the agent\'s edits', () => {
    // The agent added a line at the top while the person kept typing at the end.
    expect(rebaseDraft('# Plan\n- ship\n', '# Plan\n- ship\n- demo\n', 'Goals\n# Plan\n- ship\n')).toBe('Goals\n# Plan\n- ship\n- demo\n')
    // Typed before what the agent changed: it keeps its place.
    expect(rebaseDraft('a b c', 'a! b c', 'a b C!')).toBe('a! b C!')
    expect(rebaseDraft('same', 'same', 'same, merged')).toBe('same, merged')
  })
  it('names columns as spreadsheets do', () => {
    expect([0, 25, 26, 701].map(columnName)).toEqual(['A', 'Z', 'AA', 'ZZ'])
  })
})
