import type { PromptAttachment } from '../api'
import { Icon } from '../ui'

export function AttachedFiles({ files, onRemove }: { files: PromptAttachment[]; onRemove: (index: number) => void }) {
  if (files.length === 0) return null
  return (
    <ul aria-label="attached files" className="flex flex-wrap gap-1.5 px-3 pt-2.5 text-xs">
      {files.map((file, index) => (
        <li key={`${file.name}-${index}`} className="flex items-center gap-1.5 rounded-sm border border-line bg-surface-1 px-2 py-1">
          <Icon name={file.mime.startsWith('image/') ? 'image' : 'file'} className="h-3.5 w-3.5 text-fg-2" />
          {file.name}
          <button type="button" aria-label={`remove ${file.name}`} className="text-fg-2 hover:text-fg-1" onClick={() => onRemove(index)}>
            <Icon name="x" className="h-3 w-3" />
          </button>
        </li>
      ))}
    </ul>
  )
}
