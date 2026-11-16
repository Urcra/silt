import { describe, expect, it } from 'vitest'
import { canvasOf, chartColors, chartTable, pointer, runs, stepsOf, toolValue, withLive, xPositions, yDomain, columnName, csvOf, dataPageOf, deckOf, layout, parsed, partDocument, rebaseDraft, shown, slideFromHash, splice, drawnSeries } from './typed'

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
    expect(doc).toContain('color-scheme:dark;background:var(--bg,Canvas)')
    const imported = partDocument('', "h1{color:red}@import url('https://fonts.googleapis.com/css2?family=Inter');", 1280, 720, null, 'light')
    expect(imported.indexOf('@import')).toBeLessThan(imported.indexOf('html,body'))
    expect(parsed('{bad')).toBeNull()
    expect(stepsOf('<li data-step="1">a</li><li data-step=\'3\'>b</li><p>c</p>')).toBe(3)
    const stepped = partDocument('<li data-step="1">a</li>', undefined, 1280, 720, null, 'light', 1)
    expect(stepped).toContain('[data-step]{visibility:hidden}[data-step="1"]{visibility:visible}')
    expect(partDocument('<li data-step="1">a</li>', undefined, 1280, 720, null, 'light')).not.toContain('visibility')
    const grows = partDocument('<p>long</p>', undefined, 390, 844, null, 'light', undefined, true)
    expect(grows).toContain('min-height:844px')
    expect(grows).toContain("silt:'grown'")
  })
})

describe("a data page's live numbers", () => {
  it("come from where its blocks point in the tool's answer, or stay as written", () => {
    const answer = toolValue({ content: [{ type: 'text', text: '{"open": 12, "by": [["mon", 3], ["tue", 5]], "rows": [["a", 1]]}' }] })
    expect(pointer(answer, '/by/1/1')).toBe(5)
    expect(toolValue({ structuredContent: { open: 1 }, content: [] })).toEqual({ open: 1 })
    const page = {
      title: 't',
      sources: [{ id: 'live', name: 'tracker', kind: 'mcp', detail: '', at: '', server: 'tracker', tool: 'count' }],
      blocks: [
        { type: 'stat' as const, label: 'Open', value: 0, source: 'live', pick: '/open' },
        { type: 'table' as const, columns: ['who', 'n'], rows: [], source: 'live', pick: '/rows' },
        { type: 'chart' as const, chart: 'bar', series: [{ name: 'by day', points: [], pick: '/by' }], source: 'live' },
        { type: 'stat' as const, label: 'Missing', value: 7, source: 'live', pick: '/none' },
      ],
    }
    const shown = withLive(page, { live: answer })
    expect(shown.blocks[0]).toMatchObject({ value: 12 })
    expect(shown.blocks[1]).toMatchObject({ rows: [['a', 1]] })
    expect(shown.blocks[2]).toMatchObject({ series: [{ points: [['mon', 3], ['tue', 5]] }] })
    expect(shown.blocks[3]).toMatchObject({ value: 7 })
  })
})

describe('a chart past six series', () => {
  const many = Array.from({ length: 8 }, (_, at) => ({ name: `s${at}`, points: [['mon', at + 1], ['tue', at === 0 ? null : 1]] as [string, number | null][] }))
  it('folds the smallest into Other where its series add up, keeping the page order', () => {
    const drawn = drawnSeries('bar', many)
    expect(drawn.map((one) => one.name)).toEqual(['s3', 's4', 's5', 's6', 's7', 'Other (3)'])
    expect(drawn[5].points).toEqual([['mon', 6], ['tue', 2]])
  })
  it('keeps them all on a line chart, and six or fewer anywhere', () => {
    expect(drawnSeries('line', many)).toHaveLength(8)
    expect(drawnSeries('bar', many.slice(0, 6))).toHaveLength(6)
  })
})

describe("a chart's scales", () => {
  it('place x by order, by number, or by date', () => {
    expect(xPositions(['a', 'b', 'c'], undefined)).toEqual([0, 0.5, 1])
    expect(xPositions([0, 10, 2.5], 'number')).toEqual([0, 1, 0.25])
    expect(xPositions(['2026-11-01', '2026-11-11', '2026-11-03'], 'time')).toEqual([0, 1, 0.2])
    expect(xPositions(['a', 2], 'number')).toEqual([0, 1])
  })
  it('break a line where a value is missing, and start y near the values only when asked', () => {
    expect(runs([1, 2, null, 4, undefined, null, 6])).toEqual([[[0, 1], [1, 2]], [[3, 4]], [[6, 6]]])
    expect(yDomain([400, 500], true)).toEqual([0, 500])
    const [low, high] = yDomain([400, 500], false)
    expect(low).toBeGreaterThan(300)
    expect(high).toBeLessThan(600)
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
