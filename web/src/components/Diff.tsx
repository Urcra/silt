import { lineClass } from '../diffStyle'
import { diffFile } from '../permalink'

/** A unified diff; with `anchors`, each file's header is `#file-<path>` (ADR 0031). */
export function Diff({ text, anchors = false, picked }: { text: string; anchors?: boolean; picked?: string }) {
  if (!text.trim()) return <p className="text-sm text-fg-3">No changes.</p>
  return (
    <pre data-testid="diff" className="overflow-x-auto rounded-md border border-line font-mono text-xs leading-5">
      {text.replace(/\n$/, '').split('\n').map((line, index) => {
        const file = anchors ? diffFile(line) : null
        return (
          <div
            key={index}
            id={file === null ? undefined : `file-${file}`}
            className={`px-2 ${lineClass(line)} ${file !== null && file === picked ? 'ring-2 ring-accent' : ''}`}
          >
            {line || ' '}
          </div>
        )
      })}
    </pre>
  )
}
