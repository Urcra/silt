// Where a doc's commented text is in its rendered read view, for highlighting it (ADR 0082).

/** A place in a list of text nodes' texts: which node, and how far into it. */
export type TextPlace = [node: number, offset: number]

/** What a comment's excerpt reads as once rendered: Markdown's emphasis, code ticks, heading marks and link targets gone. */
export function renderedExcerpt(excerpt: string): string {
  return excerpt
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}\s+|[-*+]\s+|\d+[.)]\s+|>\s?)/gm, '')
    .replace(/[*_`~]+/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}

/**
 * Where `needle` first occurs across `texts`, the text nodes of a rendered view in order, any run of whitespace in
 * either matching any other; null when it doesn't, or is empty.
 */
export function findAcross(texts: string[], needle: string): { start: TextPlace; end: TextPlace } | null {
  const wanted = needle.replace(/\s+/g, ' ').trim()
  if (!wanted) return null
  let joined = ''
  const places: TextPlace[] = []
  texts.forEach((text, node) => {
    for (let offset = 0; offset < text.length; offset++) {
      const char = /\s/.test(text[offset]) ? ' ' : text[offset]
      if (char === ' ' && joined.endsWith(' ')) continue
      joined += char
      places.push([node, offset])
    }
  })
  const at = joined.indexOf(wanted)
  if (at < 0) return null
  const last = places[at + wanted.length - 1]
  return { start: places[at], end: [last[0], last[1] + 1] }
}
