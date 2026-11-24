import { type Attached, uploading } from '../attachments'
import { sizeOf } from '../mediaInfo'
import { Icon, cx } from '../ui'

/**
 * The files a prompt carries, each with its upload's progress and, for an image or a video, a glimpse of it
 * (ADR 0120). A note says what the agent gets of a video, which no agent takes in a prompt.
 */
export function AttachedFiles({ files, onRemove }: { files: Attached[]; onRemove: (index: number) => void }) {
  if (files.length === 0) return null
  const video = files.some((file) => file.mime.startsWith('video/'))
  return (
    <>
      <ul aria-label="attached files" className="flex flex-wrap gap-1.5 px-3 pt-2.5 text-xs">
        {files.map((file, index) => {
          const going = uploading(file)
          const share = file.size > 0 ? Math.min(100, Math.round((file.sent / file.size) * 100)) : 0
          return (
            <li
              key={file.key ?? `${file.name}-${index}`}
              className={cx(
                'relative flex items-center gap-1.5 overflow-hidden rounded-sm border bg-surface-1 py-1 pr-2',
                file.local ? 'pl-1' : 'pl-2',
                file.error ? 'border-danger' : 'border-line',
              )}
              title={file.error}
            >
              {file.local && file.mime.startsWith('video/') ? (
                <video src={file.local} muted preload="metadata" className="h-5 w-9 rounded-[2px] object-cover" />
              ) : file.local ? (
                <img src={file.local} alt="" className="h-5 w-9 rounded-[2px] object-cover" />
              ) : (
                <Icon name={file.mime.startsWith('image/') ? 'image' : 'file'} className="h-3.5 w-3.5 text-fg-2" />
              )}
              {file.name}
              {file.size > 0 && <span className="font-mono text-2xs text-fg-3">{sizeOf(file.size)}</span>}
              {going && <span className="font-mono text-2xs text-fg-3">· uploading {share}%</span>}
              {file.error && <span className="text-2xs text-danger">· didn’t upload</span>}
              <button
                type="button"
                aria-label={`remove ${file.name}`}
                className="text-fg-2 hover:text-fg-1"
                onClick={() => onRemove(index)}
              >
                <Icon name="x" className="h-3 w-3" />
              </button>
              {going && (
                <span aria-hidden="true" className="absolute bottom-0 left-0 h-0.5 bg-accent" style={{ width: `${share}%` }} />
              )}
            </li>
          )
        })}
      </ul>
      {video && (
        <p className="flex items-center gap-1.5 px-3 pt-1.5 text-2xs text-fg-2">
          <Icon name="info" className="h-3.5 w-3.5" />
          The agent gets a video’s path and can read its frames with a tool; agents don’t take video in a prompt.
        </p>
      )}
    </>
  )
}
