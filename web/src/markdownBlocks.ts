// Splitting a message being streamed into parts that parse alone as they would together.

/**
 * `text` cut at its blank lines into parts that parse alone as they would together: not inside a fenced code block,
 * and not before what carries on the part above, an indented line or a list's next item.
 */
export function markdownBlocks(text: string): string[] {
  const lines = text.split('\n')
  const blocks: string[] = []
  let current: string[] = []
  let fence: string | null = null
  lines.forEach((line, index) => {
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line)?.[1]
    if (fence === null) {
      if (marker) fence = marker
    } else if (marker && marker[0] === fence[0] && marker.length >= fence.length && line.trim() === marker) fence = null
    current.push(line)
    const next = lines[index + 1]
    if (fence !== null || line.trim() !== '' || next === undefined || next.trim() === '') return
    if (/^\s/.test(next) || /^([-*+]|\d+[.)])(\s|$)/.test(next)) return
    blocks.push(current.join('\n'))
    current = []
  })
  blocks.push(current.join('\n'))
  return blocks
}
