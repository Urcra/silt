// A doc's rich-text view (ADR 0077): its text shown as it reads, each top-level block edited in place. Leaving a block
// writes it back as Markdown in place of that block's source alone, so the rest of the text stays as it was written;
// code, diagrams, charts, tables and `<?silt …?>` blocks stay as written, and dates and mentions inside a block too.

import { useMemo, useRef } from 'react'
import TurndownService from 'turndown'
import { cx } from '../ui'
import { DocMarkdown } from './DocMarkdown'
import { docBlocks } from '../docBlocks'

const turndown = new TurndownService({
  headingStyle: 'atx',
  bulletListMarker: '-',
  codeBlockStyle: 'fenced',
  emDelimiter: '*',
  strongDelimiter: '**',
})
// Dates and mentions keep the Markdown they came from.
turndown.addRule('silt-inline', {
  filter: (node) => node instanceof HTMLElement && node.dataset.md !== undefined,
  replacement: (_content, node) => (node as HTMLElement).dataset.md ?? '',
})

/** What a block's edited DOM says, as Markdown. */
function blockMarkdown(element: HTMLElement): string {
  return turndown.turndown(element.innerHTML).trim()
}

export function RichDoc({ text, onChange, editable }: { text: string; onChange: (text: string) => void; editable: boolean }) {
  const blocks = useMemo(() => docBlocks(text), [text])
  const edited = useRef(new Set<number>())
  const commit = (at: number, element: HTMLElement) => {
    if (!edited.current.delete(at)) return
    const block = blocks[at]
    if (!block) return
    const turned = blockMarkdown(element.querySelector<HTMLElement>('[data-rich-block]') ?? element)
    const written = block.depth ? turned.replace(/^#{1,6}[ \t]*/, `${'#'.repeat(block.depth)} `) : turned
    const next = `${text.slice(0, block.start)}${written}${text.slice(block.end)}`
    if (next !== text) onChange(next)
  }
  // Each block is keyed by its source, so one written back is drawn afresh and the others keep what's typed in them.
  const seen = new Map<string, number>()
  return (
    <div className="flex flex-col gap-3" data-testid="doc-rich">
      {blocks.map((block, at) => {
        const source = text.slice(block.start, block.end)
        const twice = seen.get(source) ?? 0
        seen.set(source, twice + 1)
        const open = editable && block.editable
        return (
          <div
            key={`${twice}:${source}`}
            contentEditable={open}
            suppressContentEditableWarning
            onInput={() => edited.current.add(at)}
            onBlur={(event) => commit(at, event.currentTarget)}
            className={cx(
              'rounded-sm outline-none',
              open && 'focus:bg-surface-1/60 focus:ring-1 focus:ring-line',
              !open && 'select-text',
            )}
            aria-label={open ? 'a paragraph of the doc, editable' : undefined}
          >
            <div data-rich-block>
              <DocMarkdown text={source} />
            </div>
          </div>
        )
      })}
    </div>
  )
}
