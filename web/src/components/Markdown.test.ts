import { describe, expect, it } from 'vitest'
import { docMarkdown } from './Markdown'

describe("a doc's blocks", () => {
  it('become what the read view draws, and a malformed one is left to be dropped', () => {
    expect(docMarkdown('Due <?silt date 2026-11-20?>, ask <?silt mention ann?>.')).toBe('Due [2026-11-20](#silt-date:2026-11-20), ask [@ann](#silt-mention:ann).')
    expect(docMarkdown('<?silt chart {"chart": "bar", "series": []}?>')).toContain('```silt-chart\n{"chart": "bar", "series": []}\n```')
    expect(docMarkdown('<?silt date next week?>')).toBe('<?silt date next week?>')
    expect(docMarkdown('<?silt mention <script>?>')).toBe('<?silt mention <script>?>')
  })
})
