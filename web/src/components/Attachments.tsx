import type { PromptAttachment } from '../api'
import type { Takes } from '../attachments'
import { Icon, cx } from '../ui'
import { GHOST_CHIP } from './ChipSelect'

export function AttachedFiles({ files, onRemove }: { files: PromptAttachment[]; onRemove: (index: number) => void }) {
  if (files.length === 0) return null
  return (
    <ul aria-label="attached files" className="flex flex-wrap gap-1.5 px-4 pb-2 text-xs">
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

export function AttachButton({ takes, onFiles }: { takes: Takes; onFiles: (files: File[]) => void }) {
  return (
    <label
      className={cx(GHOST_CHIP, 'w-7 cursor-pointer justify-center px-0 text-fg-2')}
      title={takes.images === false ? `Attach text files; ${takes.agent} takes no images` : 'Attach images or files'}
    >
      <Icon name="attach" className="h-4 w-4" />
      <input
        type="file"
        multiple
        aria-label="attach files"
        className="sr-only"
        accept={takes.images === false && takes.files === false ? 'text/*,.md,.json,.ts,.tsx,.rs,.py,.toml,.yaml,.yml' : undefined}
        onChange={(event) => {
          const files = [...(event.target.files ?? [])]
          event.target.value = ''
          onFiles(files)
        }}
      />
    </label>
  )
}
