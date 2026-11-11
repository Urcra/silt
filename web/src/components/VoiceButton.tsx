import { useEffect, useState } from 'react'
import { Button, Icon, Spinner, cx } from '../ui'
import { DICTATION_KEYS, type Dictation } from '../voice'

const MODEL_MEGABYTES = 103

export function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="inline-flex gap-0.5">
      {keys.map((key) => (
        <kbd key={key} className="rounded-sm border border-b-2 border-line bg-elevated px-1 font-mono text-2xs text-fg-2">
          {key}
        </kbd>
      ))}
    </span>
  )
}

/** The microphone in the box: starts a take, and while one runs, glows and keeps its words when pressed. */
export function VoiceButton({ dictation }: { dictation: Dictation }) {
  const { phase, press } = dictation
  const live = phase === 'recording'
  const busy = phase === 'starting' || phase === 'finishing' || phase === 'downloading'
  return (
    <button
      type="button"
      aria-label={live ? 'Stop dictating' : 'Dictate'}
      title={live ? 'Stop, and keep the words (Esc throws them away)' : `Dictate, or hold ${DICTATION_KEYS.join('+')} to talk; your voice stays on this machine`}
      disabled={busy}
      onClick={() => void press()}
      className={cx(
        'inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full max-md:h-11 max-md:w-11',
        live
          ? 'mx-1 bg-accent-tint text-accent-fg shadow-[0_0_0_2px_var(--accent),0_0_0_6px_var(--accent-tint)]'
          : 'text-fg-2 hover:bg-surface-2 hover:text-fg-1 disabled:hover:bg-transparent',
        phase === 'offering' && 'bg-surface-2 text-fg-1',
      )}
    >
      {busy ? <Spinner className="h-4 w-4" /> : <Icon name="mic" className="h-4 w-4" />}
    </button>
  )
}

/** The first press: the model is downloaded only once the person agrees. Over the box, by the mic. */
export function VoiceOffer({ dictation }: { dictation: Dictation }) {
  if (dictation.phase !== 'offering') return null
  return (
    <div
      role="dialog"
      aria-label="download the speech model"
      className="absolute right-2 bottom-full z-20 mb-2 w-72 rounded-md border border-line bg-elevated p-3 text-xs text-fg-2 shadow-overlay"
    >
      <p className="mb-1 text-sm font-medium text-fg-1">Download the speech model?</p>
      Dictation needs a {MODEL_MEGABYTES} MB speech model, downloaded once. It runs on this machine, and your voice never leaves it.
      <div className="mt-2.5 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={dictation.dismissOffer}>
          Not now
        </Button>
        <Button variant="primary" size="sm" onClick={() => void dictation.startDownload()}>
          Download
        </Button>
      </div>
    </div>
  )
}

/** What came of the last take, or the download, at the left of the row under the box. */
export function VoiceNote({ dictation }: { dictation: Dictation }) {
  const { phase, problem, download } = dictation
  if (phase === 'downloading') {
    const percent = download ? Math.floor((download.done / Math.max(download.total, 1)) * 100) : 0
    return <span className="text-xs text-fg-2">Downloading the speech model… {percent}%</span>
  }
  if (problem) {
    return (
      <span role="status" className={cx('text-xs', problem.tone === 'danger' ? 'text-danger' : 'text-fg-2')}>
        {problem.text}
      </span>
    )
  }
  return null
}

function useElapsed(since: number | undefined): string {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (since === undefined) return undefined
    const tick = setInterval(() => setNow(Date.now()), 250)
    return () => clearInterval(tick)
  }, [since])
  const seconds = since === undefined ? 0 : Math.max(0, Math.floor((now - since) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

/** The levels heard, newest on the right; flat while nothing comes through. */
function Waveform({ levels, quiet }: { levels: number[]; quiet: boolean }) {
  return (
    <span aria-hidden="true" className="flex h-6 min-w-0 flex-1 items-center justify-end gap-0.5 overflow-hidden px-2">
      {levels.map((loudness, index) => (
        <i
          key={index}
          className={cx('block w-0.5 shrink-0 rounded-full', quiet ? 'bg-line-strong' : 'bg-accent')}
          style={{ height: `${Math.max(2, Math.round(loudness * 22))}px`, opacity: quiet ? 1 : 0.45 + 0.55 * (index / Math.max(levels.length, 1)) }}
        />
      ))}
    </span>
  )
}

function Round({ label, title, icon, onClick }: { label: string; title: string; icon: 'x' | 'check'; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title}
      onClick={onClick}
      className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-line-strong bg-surface-2 text-fg-1 hover:bg-surface-3 max-md:h-11 max-md:w-11"
    >
      <Icon name={icon} className="h-3.5 w-3.5" />
    </button>
  )
}

/** The row under the box while a take runs, in place of the agent's settings: discard, what's heard, keep. */
export function VoiceBar({ dictation }: { dictation: Dictation }) {
  const { phase, levels, quiet, since, holding, discard, keep } = dictation
  const elapsed = useElapsed(since)
  const hint =
    phase === 'starting' ? (
      'Starting the microphone…'
    ) : phase === 'finishing' ? (
      'Finishing…'
    ) : quiet ? (
      <span className="text-warn">No sound. Is the microphone muted?</span>
    ) : holding ? (
      <>
        release <Keys keys={DICTATION_KEYS} /> to insert
      </>
    ) : (
      <span className="max-md:hidden">
        <Keys keys={['Esc']} /> discard · <Keys keys={['⏎']} /> send
      </span>
    )
  return (
    <div role="group" aria-label="dictation" className="flex w-full min-w-0 items-center gap-1">
      <Round label="Discard dictation" title="Throw the words away (Esc)" icon="x" onClick={discard} />
      <Waveform levels={levels} quiet={quiet} />
      {phase === 'recording' && <span className="mr-1 font-mono text-2xs text-fg-2">{elapsed}</span>}
      <span className="mr-2 inline-flex items-center gap-1 text-xs whitespace-nowrap text-fg-3">{hint}</span>
      {!holding && <Round label="Keep the words" title="Stop, and put the words into the draft" icon="check" onClick={keep} />}
    </div>
  )
}
