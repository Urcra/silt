import { Button, Icon, Menu } from '../ui'
import type { Dictation } from '../voice'
import { RoundButton } from './RoundButton'

const MODEL_MEGABYTES = 103

export function VoiceButton({ dictation }: { dictation: Dictation }) {
  const { phase, autoSend, setAutoSend, press } = dictation
  const recording = phase === 'recording'
  return (
    <span className="flex items-center">
      <RoundButton
        quiet={!recording}
        label={recording ? 'Stop dictating' : 'Dictate'}
        title={recording ? 'Stop, and keep the words (Esc throws them away)' : 'Dictate; your voice stays on this machine'}
        icon={recording ? 'stop' : 'mic'}
        busy={phase === 'starting' || phase === 'finishing' || phase === 'downloading'}
        disabled={phase === 'starting' || phase === 'finishing' || phase === 'downloading'}
        onClick={() => void press()}
      />
      <Menu
        label="Dictation options"
        trigger={<Icon name="chevronDown" className="h-3.5 w-3.5" />}
        items={[{ label: `${autoSend ? '✓ ' : ''}Send when I stop talking`, onSelect: () => setAutoSend(!autoSend) }]}
      />
    </span>
  )
}

/** What's being heard, or what stands in the way of it, under the text. */
export function VoiceLine({ dictation }: { dictation: Dictation }) {
  const { phase, phrases, partial, problem, download, startDownload, dismissOffer } = dictation
  if (phase === 'offering') {
    return (
      <p className="flex flex-wrap items-center gap-2 px-4 pb-1.5 text-xs text-fg-2">
        Dictation needs a {MODEL_MEGABYTES} MB speech model, which stays on this machine.
        <Button variant="ghost" onClick={() => void startDownload()}>
          Download
        </Button>
        <Button variant="ghost" onClick={dismissOffer}>
          Not now
        </Button>
      </p>
    )
  }
  if (phase === 'downloading') {
    const percent = download ? Math.floor((download.done / Math.max(download.total, 1)) * 100) : 0
    return <p className="px-4 pb-1.5 text-xs text-fg-2">Downloading the speech model… {percent}%</p>
  }
  if (phase === 'recording' || phase === 'finishing') {
    const heard = [...phrases, partial].filter(Boolean).join(' ')
    return (
      <p aria-live="polite" aria-label="heard so far" className="px-4 pb-1.5 text-sm text-fg-3">
        {heard || 'Listening…'}
      </p>
    )
  }
  if (problem) {
    return (
      <p role="status" className="px-4 pb-1.5 text-xs text-danger">
        {problem}
      </p>
    )
  }
  return null
}
