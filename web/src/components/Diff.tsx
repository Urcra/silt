import { lineClass } from '../diffStyle'

export function Diff({ text }: { text: string }) {
  if (!text.trim()) return <p className="text-sm text-fg-3">No changes.</p>
  return (
    <pre data-testid="diff" className="overflow-x-auto rounded-md border border-line font-mono text-xs leading-5">
      {text.replace(/\n$/, '').split('\n').map((line, index) => (
        <div key={index} className={`px-2 ${lineClass(line)}`}>
          {line || ' '}
        </div>
      ))}
    </pre>
  )
}
