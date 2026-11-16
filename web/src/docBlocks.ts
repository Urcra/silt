// A doc's top-level blocks, by where each is in its source (ADR 0077), for its rich-text view to write back one at a time.

import remarkGfm from 'remark-gfm'
import remarkParse from 'remark-parse'
import { unified } from 'unified'

/** One top-level block: where it is in the source, and whether the rich view edits it in place. */
/** A heading's level is its source's, as the view draws headings smaller than they're written. */
export type DocBlock = { start: number; end: number; editable: boolean; depth?: number }

/** Prose, which turns back into Markdown faithfully; code, tables and `<?silt …?>` blocks stay as written. */
const EDITABLE = new Set(['paragraph', 'heading', 'list', 'blockquote'])

const parser = unified().use(remarkParse).use(remarkGfm)

export function docBlocks(text: string): DocBlock[] {
  return parser.parse(text).children.flatMap((node) => {
    const [start, end] = [node.position?.start.offset, node.position?.end.offset]
    if (start === undefined || end === undefined) return []
    return [{ start, end, editable: EDITABLE.has(node.type), ...(node.type === 'heading' ? { depth: node.depth } : {}) }]
  })
}
