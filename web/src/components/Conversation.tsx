import type { ShowLine } from '../api'

export function Conversation({ lines }: { lines: ShowLine[] }) {
  return (
    <div className="space-y-1 text-sm">
      {lines.map((line, index) => {
        switch (line.line) {
          case 'entry':
            if (line.kind === 'user') return <p key={index} className="mt-3 font-medium">› {line.text}</p>
            if (line.kind === 'assistant') return <p key={index} className="whitespace-pre-wrap">{line.text}</p>
            return <p key={index} className="truncate text-xs text-fg-3">{line.kind}: {line.text}</p>
          case 'tool_call':
            return <p key={index} className="font-mono text-xs text-fg-2">⚙ {line.name}</p>
          case 'edits':
            return (
              <p key={index} className="font-mono text-xs text-fg-2">
                ✎ {line.files.map((file) => `${file.path} +${file.inserted} -${file.deleted}`).join(', ')}
              </p>
            )
          case 'compaction':
            return <p key={index} className="text-center text-xs text-accent-fg">── context compacted ──</p>
          case 'segment':
            return (
              <h4 key={index} className="mt-4 font-mono text-[11px] tracking-wider text-fg-3 uppercase">
                {line.label}
              </h4>
            )
          case 'event':
            return <p key={index} className="text-sm text-info">⇄ {line.text}</p>
          default:
            return null
        }
      })}
    </div>
  )
}
