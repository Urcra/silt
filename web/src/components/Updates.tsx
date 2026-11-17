// Settings › Updates (ADR 0098): which build this is, whether a newer one is
// out, and an update's progress, as the runner writes it, through the
// daemon's restart.

import { useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { type BuildView, type NewCommit, type StepView, type UpdateView, api } from '../api'
import { inDesktop, relaunchDesktop } from '../desktop'
import { useNow } from '../now'
import { keys, message } from '../query'
import { SETTINGS_HEADING } from '../settingsContext'
import { ago } from '../time'
import { Button, Dot, Icon, Spinner, Tag, cx, toast } from '../ui'
import { inFlight, offReason, sameCommit, took, updateState, useUpdate } from '../updates'

/** Text with `code` spans, as the reasons are written. */
function Ticked({ text }: { text: string }) {
  return (
    <>
      {text.split('`').map((part, index) =>
        index % 2 === 1 ? (
          <code key={index} className="font-mono text-xs">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  )
}

const short = (commit: string | null | undefined) => commit?.slice(0, 7) ?? ''

function Code({ children }: { children: ReactNode }) {
  return <code className="rounded-sm border border-line-subtle bg-surface-1 px-1 font-mono text-xs text-fg-1">{children}</code>
}

function BuildCard({ build }: { build: BuildView }) {
  const stable = build.channel === 'stable'
  return (
    <div className="flex flex-col gap-1 rounded-md border border-line bg-elevated px-4 py-3 text-sm" data-testid="update-build">
      <div className="flex flex-wrap items-center gap-2">
        <Tag tone={stable ? 'neutral' : 'accent'} className="font-sans">
          {stable ? 'Stable' : 'Nightly'}
        </Tag>
        {stable ? (
          <span>
            <Code>v{build.version}</Code> · installed from a release
          </span>
        ) : (
          <span>
            built from <Code>{short(build.commit) || 'an unknown commit'}</Code>
            {build.branch && (
              <>
                {' '}
                on <Code>{build.branch}</Code>
              </>
            )}
            {build.dirty && ' with local changes'}
          </span>
        )}
      </div>
      {!stable && build.source && <span className="font-mono text-2xs text-fg-3">{build.source}</span>}
    </div>
  )
}

function Commits({ commits, ahead, now }: { commits: NewCommit[]; ahead: number; now: number }) {
  const [all, setAll] = useState(false)
  const shown = all ? commits : commits.slice(0, 5)
  const more = ahead - shown.length
  return (
    <div className="flex flex-col rounded-md border border-line bg-elevated text-sm" data-testid="update-commits">
      <div className="px-4 pt-3 pb-2 font-medium">
        <Code>main</Code> is {ahead} {ahead === 1 ? 'commit' : 'commits'} ahead
      </div>
      <ul className="flex flex-col pb-1.5">
        {shown.map((commit) => (
          <li key={commit.hash} className="flex items-baseline gap-2.5 px-4 py-1">
            <span className="shrink-0 font-mono text-2xs text-fg-3">{short(commit.hash)}</span>
            <span className="min-w-0 flex-1 truncate">{commit.subject}</span>
            <span className="shrink-0 text-2xs text-fg-3">{ago(commit.time * 1000, now)}</span>
          </li>
        ))}
      </ul>
      {more > 0 &&
        (shown.length < commits.length ? (
          <button type="button" className="self-start px-4 pb-3 text-xs text-accent-fg hover:underline" onClick={() => setAll(true)}>
            and {more} more
          </button>
        ) : (
          <span className="px-4 pb-3 text-xs text-fg-3">and {more} more</span>
        ))}
    </div>
  )
}

function Point({ tone, children }: { tone: 'ok' | 'accent' | 'idle'; children: ReactNode }) {
  return (
    <li className="flex items-start gap-2.5">
      <Dot tone={tone} className="mt-[7px] ml-1 mr-0.5" />
      <span>{children}</span>
    </li>
  )
}

function StepIcon({ step }: { step: StepView }) {
  switch (step.status) {
    case 'done':
      return <Icon name="check" className="h-[15px] w-[15px] text-ok" aria-label="done" />
    case 'active':
      return <Spinner label="running" className="h-3 w-3" />
    case 'waiting':
      return <Icon name="clock" className="h-[15px] w-[15px] text-accent-fg" aria-label="waiting" />
    case 'failed':
      return <Icon name="x" className="h-[15px] w-[15px] text-danger" aria-label="failed" />
    default:
      return <span role="img" aria-label="not yet" className="h-[9px] w-[9px] rounded-full border-[1.5px] border-line-strong" />
  }
}

function Steps({ view }: { view: UpdateView }) {
  const run = view.run
  if (!run) return null
  const waiting = run.outcome.kind === 'waiting' ? run.outcome.agents : 0
  const headline = run.outcome.kind === 'waiting' ? 'Installed' : 'Updating to'
  const left = run.expected_ms !== null && run.outcome.kind === 'running' ? Math.max(0, run.expected_ms - run.elapsed_ms) : undefined
  const meta =
    run.outcome.kind === 'running'
      ? [`${run.percent}%`, took(run.elapsed_ms), left !== undefined && left > 0 ? `about ${Math.max(1, Math.round(left / 60_000))}m left` : undefined]
          .filter(Boolean)
          .join(' · ')
      : run.outcome.kind === 'waiting'
        ? `took ${took(run.elapsed_ms)} · restart pending`
        : `stopped at ${took(run.elapsed_ms)}`
  return (
    <div className="flex flex-col gap-3 rounded-md border border-line bg-elevated px-4 py-3.5 text-sm" data-testid="update-steps">
      <div className="flex flex-wrap items-baseline gap-2">
        <span className="font-medium">
          {headline} {run.to && <Code>{short(run.to)}</Code>}
        </span>
        <span className="flex-1" />
        <span className="text-2xs text-fg-3">{meta}</span>
      </div>
      <div
        className="h-1 overflow-hidden rounded-full bg-surface-2"
        role="progressbar"
        aria-label="update progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={run.percent}
      >
        <div className="h-full rounded-full bg-accent transition-[width] duration-150" style={{ width: `${run.percent}%` }} />
      </div>
      <ol className="flex flex-col">
        {run.steps.map((step) => {
          const compiling = step.status === 'active' && step.compiled > 0
          const detail =
            step.status === 'done' && step.took_ms !== null
              ? took(step.took_ms)
              : step.status === 'waiting'
                ? `waiting for ${waiting} ${waiting === 1 ? 'agent' : 'agents'}`
                : step.status === 'failed'
                  ? `failed after ${took(step.took_ms ?? 0)}`
                  : step.status === 'skipped'
                    ? 'skipped'
                    : step.status === 'active' && !compiling
                      ? 'running'
                      : ''
          return (
            <li key={step.phase} className="flex flex-col gap-1 py-1.5" data-step={step.phase} data-status={step.status}>
              <div className="flex items-center gap-2.5">
                <span className="inline-flex h-4 w-4 shrink-0 items-center justify-center">
                  <StepIcon step={step} />
                </span>
                <span
                  className={cx(
                    'min-w-0 flex-1',
                    step.status === 'todo' || step.status === 'skipped' ? 'text-fg-3' : step.status === 'failed' ? 'text-danger' : 'text-fg-1',
                    (step.status === 'active' || step.status === 'waiting') && 'font-medium',
                  )}
                >
                  {step.label}
                </span>
                <span className={cx('text-2xs', step.status === 'failed' ? 'text-danger' : step.status === 'waiting' ? 'text-accent-fg' : 'text-fg-3')}>{detail}</span>
              </div>
              {compiling && (
                <div className="flex items-center gap-2.5 pl-[26px]">
                  {step.expected_compiled !== null && (
                    <div className="h-1 max-w-[260px] flex-1 overflow-hidden rounded-full bg-surface-2">
                      <div className="h-full rounded-full bg-accent" style={{ width: `${Math.min(100, (100 * step.compiled) / step.expected_compiled)}%` }} />
                    </div>
                  )}
                  <span className="font-mono text-2xs text-fg-3">
                    Compiling {step.compiled}
                    {step.expected_compiled !== null && ` of ~${step.expected_compiled}`} crates
                  </span>
                </div>
              )}
            </li>
          )
        })}
      </ol>
    </div>
  )
}

function Output({ lines, open, failed }: { lines: string[]; open: boolean; failed: boolean }) {
  const end = useRef<HTMLPreElement>(null)
  useEffect(() => {
    if (end.current) end.current.scrollTop = end.current.scrollHeight
  })
  if (lines.length === 0) return null
  const error = (line: string) => failed && /^\s*(error|-->|fatal)/i.test(line)
  return (
    <details open={open} className="flex flex-col gap-2">
      <summary className="cursor-pointer pb-2 text-xs text-fg-2">Output</summary>
      <pre
        ref={end}
        className="max-h-[320px] overflow-auto rounded-md border border-line bg-surface-1 px-3 py-2.5 font-mono text-[11.5px] leading-relaxed text-fg-2"
        data-testid="update-output"
      >
        {lines.map((line, index) => (
          <span key={index} className={cx('block', error(line) && 'text-danger')}>
            {line || ' '}
          </span>
        ))}
      </pre>
    </details>
  )
}

/** Settings › Updates. */
export function UpdatesSettings() {
  const queries = useQueryClient()
  const update = useUpdate()
  const now = useNow(30_000)
  const [busy, setBusy] = useState<'check' | 'start' | 'restart'>()
  const [error, setError] = useState<string>()
  const view = update.data
  const state = updateState(view)
  const refresh = () => void queries.invalidateQueries({ queryKey: keys.update })
  const act = (doing: 'check' | 'start' | 'restart', call: () => Promise<unknown>) => {
    setBusy(doing)
    setError(undefined)
    call().then(
      () => {
        refresh()
        // The runner takes a moment to write its first state.
        if (doing === 'start') window.setTimeout(refresh, 600)
      },
      (failure: unknown) => setError(message(failure)),
    )
    window.setTimeout(() => setBusy(undefined), doing === 'start' ? 1500 : 400)
  }
  // The daemon stops part-way and at the end: what it last said stands until the new one answers.
  const restarting = update.isError && view !== undefined && inFlight(view.run)

  if (!view) {
    return (
      <section aria-label="Updates" className="flex flex-col gap-3">
        <h2 className={SETTINGS_HEADING}>Updates</h2>
        {update.isError ? <p className="text-xs text-danger">{message(update.error)}</p> : <p className="text-sm text-fg-2">Looking…</p>}
      </section>
    )
  }
  const availability = view.availability
  const checked = view.checking ? 'checking…' : view.checked_ms ? `checked ${ago(view.checked_ms, now)}${ago(view.checked_ms, now) === 'just now' ? '' : ' ago'}` : 'not checked yet'
  const showCheck = !inFlight(view.run) && availability?.kind !== 'off'
  const run = view.run
  const waitingAgents = run?.outcome.kind === 'waiting' ? run.outcome.agents : 0
  return (
    <section aria-label="Updates" className="flex flex-col gap-3.5" data-state={state}>
      <div className="flex min-h-7 items-center gap-2.5">
        <h2 className={SETTINGS_HEADING}>Updates</h2>
        <span className="flex-1" />
        {showCheck && (
          <>
            <span className="text-2xs text-fg-3">{checked}</span>
            <Button icon="reload" onClick={() => act('check', api.checkUpdate)} disabled={view.checking || busy === 'check'}>
              Check now
            </Button>
          </>
        )}
      </div>
      <BuildCard build={view.build} />

      {state === 'up-to-date' && (
        <div className="flex items-center gap-2 px-0.5 text-sm">
          <Dot tone="ok" />
          <span>{view.build.channel === 'stable' ? 'This is the latest release.' : <>Up to date with <Code>main</Code></>}</span>
        </div>
      )}

      {state === 'behind' && availability?.kind === 'behind' && (
        <>
          <Commits commits={availability.commits} ahead={availability.ahead} now={now} />
          <div className="flex flex-col gap-2 px-0.5 text-sm">
            <p className="text-xs font-medium text-fg-2">What happens</p>
            <ul className="flex flex-col gap-1.5">
              <Point tone="idle">
                Fast-forwards <Code>main</Code>
                {view.build.source && (
                  <>
                    {' '}
                    in <Code>{view.build.source}</Code>
                  </>
                )}{' '}
                and runs <Code>just update</Code> there.
              </Point>
              <Point tone="idle">The daemon restarts on the new build. Agents keep running, and this window follows the daemon.</Point>
              {inDesktop() &&
                (view.relaunch ? (
                  <Point tone="accent">
                    The desktop app changed since this build (<Code>crates/silt-desktop</Code>), so you relaunch it at the end.
                  </Point>
                ) : (
                  <Point tone="idle">The desktop app hasn't changed, so there's no relaunch.</Point>
                ))}
            </ul>
          </div>
          <div className="flex items-center gap-3 pt-1">
            <Button variant="primary" size="lg" icon="arrowUp" onClick={() => act('start', api.startUpdate)} disabled={busy === 'start'}>
              Update now
            </Button>
            <span className="text-2xs text-fg-3">you can keep working</span>
          </div>
        </>
      )}

      {(state === 'running' || state === 'waiting' || state === 'failed') && <Steps view={view} />}

      {restarting && (
        <div className="flex items-center gap-2 text-sm text-fg-2" role="status">
          <Spinner />
          The daemon is restarting; this page picks up where it was once it's back.
        </div>
      )}

      {state === 'waiting' && (
        <div className="flex flex-col gap-2.5 rounded-md bg-accent-tint px-4 py-3 text-sm" role="status">
          <span>
            <span className="font-medium">Installed.</span> {waitingAgents} {waitingAgents === 1 ? 'agent is' : 'agents are'} mid-turn, so the daemon restarts on the new build once
            they finish.
          </span>
          <div className="flex flex-wrap items-center gap-2.5">
            <Button variant="danger" onClick={() => act('restart', api.restartForUpdate)} disabled={busy === 'restart'}>
              Restart now
            </Button>
            <span className="text-2xs text-fg-3">ends their turns; the threads keep what they recorded</span>
          </div>
        </div>
      )}

      {state === 'failed' && run?.outcome.kind === 'failed' && (
        <div className="flex flex-col gap-1.5 rounded-md bg-danger-tint px-4 py-3 text-sm" role="alert">
          <span className="font-medium text-danger">
            <Ticked text={run.outcome.message} />
          </span>
          <span className="text-fg-1">
            Silt still runs <Code>{short(view.build.commit)}</Code>. Whatever the update installed before it stopped stays installed.
          </span>
        </div>
      )}

      {(state === 'running' || state === 'failed') && <Output lines={view.output} open={state === 'failed'} failed={state === 'failed'} />}

      {state === 'failed' && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={() => act('start', api.startUpdate)} disabled={busy === 'start'}>
            Retry
          </Button>
          <Button
            onClick={() =>
              void navigator.clipboard.writeText(view.output.join('\n')).then(
                () => toast('Copied the output', 'ok'),
                () => toast('Could not copy the output', 'danger'),
              )
            }
          >
            Copy output
          </Button>
          <span className="text-2xs text-fg-3">
            full output in <span className="font-mono">{view.output_path}</span>
          </span>
        </div>
      )}

      {state === 'running' && <p className="text-2xs text-fg-3">You can keep working. If the daemon restarts partway, this page picks up where it was.</p>}

      {(state === 'done' || state === 'relaunch') && run?.outcome.kind === 'done' && sameCommit(run.to, view.build.commit) && (
        <div className="flex items-center gap-2.5 rounded-md bg-ok-tint px-4 py-3 text-sm" role="status">
          <Icon name="check" className="shrink-0 text-ok" />
          <span className="flex-1">
            <span className="font-medium">
              Updated to <Code>{short(run.to)}</Code>.
            </span>{' '}
            The daemon restarted on the new build.
          </span>
          <span className="text-2xs text-fg-3">took {took(run.elapsed_ms)}</span>
        </div>
      )}

      {state === 'relaunch' && (
        <div className="flex flex-wrap items-center gap-3.5 rounded-md border border-line bg-elevated px-4 py-3.5 text-sm">
          <div className="flex min-w-[260px] flex-1 flex-col gap-0.5">
            <span className="font-medium">Relaunch the desktop app to finish</span>
            <span className="text-xs text-fg-2">Its own code changed. Threads and agents carry on through the relaunch.</span>
          </div>
          <Button variant="primary" size="lg" icon="reload" onClick={relaunchDesktop}>
            Relaunch Silt
          </Button>
        </div>
      )}

      {state === 'done' && (
        <div className="flex items-center gap-2 px-0.5 text-sm text-fg-2">
          <Dot tone="idle" />
          <span>{inDesktop() ? 'No relaunch needed: this window already shows the new build.' : 'Reload other tabs to see the new web UI.'}</span>
        </div>
      )}

      {state === 'off' && availability?.kind === 'off' && (
        <div className="flex flex-col gap-2.5 rounded-md bg-info-tint px-4 py-3 text-sm">
          <span>
            <span className="font-medium">Updates are off for this build.</span> <Ticked text={offReason(availability.reason)} />
          </span>
          {view.build.source && availability.reason.kind !== 'debug_build' && (
            <>
              <span className="text-xs text-fg-2">
                To turn them back on, install from a clean <Code>main</Code> in <Code>{view.build.source}</Code>:
              </span>
              <pre className="rounded-md border border-line bg-elevated px-3 py-2 font-mono text-xs text-fg-1">git switch main && git pull && just install</pre>
            </>
          )}
        </div>
      )}

      {state === 'release' && availability?.kind === 'release' && (
        <>
          <div className="flex flex-wrap items-center gap-3 rounded-md border border-line bg-elevated px-4 py-3.5 text-sm">
            <div className="flex min-w-[240px] flex-1 flex-col gap-0.5">
              <span className="font-medium">
                <Code>{availability.tag}</Code> is out
              </span>
              <span className="text-2xs text-fg-3">you have v{view.build.version}</span>
            </div>
            <a className="inline-flex items-center gap-1.5 rounded-sm border border-transparent bg-accent px-2.5 py-1 text-xs font-medium text-on-accent hover:bg-accent-hover" href={availability.url} target="_blank" rel="noreferrer">
              <Icon name="download" />
              Download
            </a>
          </div>
          <p className="text-2xs text-fg-3">A stable release installs with its installer for now. Updating in place comes with signed releases.</p>
        </>
      )}

      {availability?.kind === 'failed' && !inFlight(run) && (
        <p className="text-xs text-danger">
          Could not check: <Ticked text={availability.message} />
        </p>
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

