import type { ShowLine } from '../api'

export function Conversation({ lines }: { lines: ShowLine[] }) {
  return (
    <div className="space-y-1 text-sm">
      {lines.map((line, index) => {
        switch (line.line) {
          case 'entry':
            if (line.kind === 'user') return <p key={index} className="mt-3 font-medium">› {line.text}</p>
            if (line.kind === 'assistant') return <p key={index} className="whitespace-pre-wrap">{line.text}</p>
            return <p key={index} className="truncate text-xs text-neutral-500">{line.kind}: {line.text}</p>
          case 'tool_call':
            return <p key={index} className="text-xs text-neutral-600">⚙ {line.name}</p>
          case 'edits':
            return (
              <p key={index} className="text-xs text-neutral-600">
                ✎ {line.files.map((file) => `${file.path} +${file.inserted} -${file.deleted}`).join(', ')}
              </p>
            )
          case 'compaction':
            return <p key={index} className="text-center text-xs text-amber-700">── context compacted ──</p>
          case 'segment':
            return <h4 key={index} className="mt-4 text-xs font-semibold uppercase text-neutral-500">{line.label}</h4>
          case 'event':
            return <p key={index} className="text-sm text-violet-700">⇄ {line.text}</p>
          default:
            return null
        }
      })}
    </div>
  )
}
