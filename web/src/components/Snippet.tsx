/** A search hit's snippet, whose matches the index brackets as `[word]`, with each match marked. */
export function Snippet({ text }: { text: string }) {
  return (
    <>
      {text.split(/\[([^\]]*)\]/).map((part, index) =>
        index % 2 === 1 ? (
          <mark key={index} className="rounded-sm border-b border-accent bg-accent-tint text-fg-1">
            {unmarked(part)}
          </mark>
        ) : (
          unmarked(part)
        ),
      )}
    </>
  )
}

/** A snippet's text without the Markdown its message was written in, which a line out of context shows as stray marks. */
function unmarked(text: string): string {
  return text.replace(/`+|\*\*/g, '')
}
