/** A search hit's snippet, whose matches the index brackets as `[word]`, with each match marked. */
export function Snippet({ text }: { text: string }) {
  return (
    <>
      {text.split(/\[([^\]]*)\]/).map((part, index) =>
        index % 2 === 1 ? (
          <mark key={index} className="rounded-sm bg-accent-tint text-fg-1">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  )
}
