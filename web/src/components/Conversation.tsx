import type { ShowLine } from '../api'
import { Counts, Icon } from '../ui'
import { Markdown } from './Markdown'

/** Lines that are the agent's tool work, folded together as on a thread's page. */
function isWork(line: ShowLine): boolean {
  return line.line === 'tool_call' || line.line === 'edits' || (line.line === 'entry' && line.kind !== 'user' && line.kind !== 'assistant')
}

function Work({ lines }: { lines: ShowLine[] }) {
  const tools = lines.filter((line) => line.line === 'tool_call')
  const edited = lines.flatMap((line) => (line.line === 'edits' ? line.files : []))
  const summary = [
    tools.length > 0 ? `Ran ${tools.length} ${tools.length === 1 ? 'tool' : 'tools'}` : 'Tool output',
    edited.length > 0 && `edited ${[...new Set(edited.map((file) => file.path.split('/').pop()))].join(', ')}`,
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <details className="group/tools text-xs text-fg-2">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 py-0.5 hover:text-fg-1 max-md:min-h-11">
        <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/tools:rotate-90" />
        <Icon name="terminal" className="h-3.5 w-3.5" />
        {summary}
      </summary>
      <div className="mt-1 ml-[18px] flex flex-col gap-1 border-l border-line pl-3">
        {lines.map((line, index) =>
          line.line === 'tool_call' ? (
            <p key={index} className="font-mono">
              <span className="font-medium text-fg-1">{line.name}</span>
            </p>
          ) : line.line === 'edits' ? (
            <p key={index} className="flex flex-wrap gap-x-3 font-mono">
              {line.files.map((file) => (
                <span key={file.path} className="inline-flex items-center gap-1.5">
                  {file.path} <Counts added={file.inserted} removed={file.deleted} />
                </span>
              ))}
            </p>
          ) : line.line === 'entry' ? (
            <p key={index} className="line-clamp-3 font-mono whitespace-pre-wrap">
              {line.text}
            </p>
          ) : null,
        )}
      </div>
    </details>
  )
}

/** A conversation as `silt show` tells it: prompts, the agent's words as Markdown, and its tool work folded. */
export function Conversation({ lines }: { lines: ShowLine[] }) {
  const pieces: (ShowLine | ShowLine[])[] = []
  for (const line of lines) {
    const last = pieces[pieces.length - 1]
    if (isWork(line)) {
      if (Array.isArray(last)) last.push(line)
      else pieces.push([line])
    } else pieces.push(line)
  }
  return (
    <div className="flex flex-col gap-2 text-sm">
      {pieces.map((piece, index) => {
        if (Array.isArray(piece)) return <Work key={index} lines={piece} />
        const line = piece
        switch (line.line) {
          case 'entry':
            if (line.kind === 'user') return <p key={index} className="mt-3 font-medium whitespace-pre-wrap">› {line.text}</p>
            return <Markdown key={index} text={line.text} />
          case 'compaction':
            return (
              <div key={index} role="separator" className="my-1 rounded-md border border-dashed border-line-strong bg-surface-1 px-3 py-1.5 text-xs">
                Context compacted here.
              </div>
            )
          case 'segment':
            return (
              <h4 key={index} className="mt-4 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
                {line.label}
              </h4>
            )
          case 'event':
            return (
              <p key={index} className="flex items-center gap-1.5 text-xs text-info">
                <Icon name="merge" className="h-3.5 w-3.5" />
                {line.text}
              </p>
            )
          default:
            return null
        }
      })}
    </div>
  )
}
