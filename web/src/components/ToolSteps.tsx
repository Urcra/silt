import { useContext, useEffect, useMemo, useState } from 'react'
import { api, type HistoryEntry, type QuestionAsked, type QuestionView } from '../api'
import { ArtifactActions } from '../artifactActions'
import { entryMedia, formatOf, sizeOf, thumbSrc } from '../mediaInfo'
import { KeptHereOnly, MediaViewer } from './Media'
import { LiveQuestions } from '../liveQuestions'
import { message } from '../query'
import { duration, imageSize, kindOf, proposedPlan, shortCommand, tailOf, testsOf, toolOf, type ToolStep } from '../transcript'
import { LiveTool, TranscriptDetail } from '../detail'
import { useNow } from '../now'
import { Button, Icon, Kbd, Spinner, cx } from '../ui'
import { changeOf } from '../edits'
import { EditCard } from './EditCard'
import { Markdown } from './Markdown'
import { linesOf, useWhole } from '../wholeEntry'

/** A picture or a video a prompt or a tool carried, small, opening in the viewer (ADR 0120). */
export function ImageThumb({ entry: given, from }: { entry: HistoryEntry; from: string }) {
  const [open, setOpen] = useState<{ at?: number } | null>(null)
  const thread = useContext(ArtifactActions)?.thread
  // A page of history leaves the picture out; it's read once its row is drawn.
  const entry = useWhole(given, true)
  const image = entry.image
  if (!image) return null
  if (!image.data && !image.stored && entry.cut)
    return (
      <span aria-busy="true" aria-label={`an image ${from}`} className="block h-24 w-40 animate-pulse rounded-md bg-surface-2" />
    )
  const media = entryMedia(thread, image, image.stored?.name ?? `An image ${from}`)
  if (!media) return <span className="text-2xs text-fg-2">An image too large to show here ({image.media_type})</span>
  const size = image.stored ? image.stored.size : (image.data.length * 3) / 4
  const facts = `${formatOf(image.media_type)} · ${image.stored ? sizeOf(size) : imageSize(image.data)} · ${from}`
  const video = media.kind === 'video'
  return (
    <figure className="flex w-fit flex-col gap-1">
      <button
        type="button"
        aria-label={`open the ${video ? 'video' : 'image'} ${from}`}
        onClick={() => setOpen({})}
        className="relative w-fit cursor-zoom-in overflow-hidden rounded-md border border-line bg-surface-1 hover:border-line-strong"
      >
        {video ? (
          <>
            <video src={media.src} preload="metadata" muted playsInline className="block max-h-44 max-w-[320px]" />
            <span className="absolute inset-0 m-auto grid h-10 w-10 place-items-center rounded-full border border-white/20 bg-[#140e08]/60 text-[#f4ecdd]">
              <Icon name="playSolid" className="ml-0.5 h-4.5 w-4.5" />
            </span>
          </>
        ) : (
          <img
            src={image.stored && thread ? thumbSrc(thread, image.stored.blob, image.stored.previews) : media.src}
            alt={media.title}
            className="block max-h-44 max-w-[320px] object-contain"
          />
        )}
      </button>
      <figcaption className="text-2xs text-fg-2">{facts}</figcaption>
      <KeptHereOnly state={media.state} />
      {open && <MediaViewer media={media} onClose={() => setOpen(null)} />}
    </figure>
  )
}

/** A step's icon: done, failed, or still waiting for its result. */
function Mark({ step }: { step: ToolStep }) {
  if (step.result?.outcome?.failed)
    return (
      <span role="img" aria-label="failed" className="mt-0.5 shrink-0 text-danger">
        <Icon name="x" className="h-3.5 w-3.5" />
      </span>
    )
  if (!step.result) return <span aria-hidden="true" className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-fg-3" />
  return <Icon name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok" />
}

const VERB: Record<string, string> = {
  read: 'Read',
  search: 'Searched',
  edit: 'Edited',
  execute: 'Ran',
  question: 'Asked',
  fetch: 'Fetched',
}
const DOING: Record<string, string> = {
  read: 'Reading',
  search: 'Searching',
  edit: 'Editing',
  execute: 'Running',
  question: 'Asking',
  fetch: 'Fetching',
}

/** What a step did, in its tool's words: `Read src/upload.rs`, `Searched progress in src`; `running`, what it's doing. */
function Said({ call, running = false }: { call: HistoryEntry; running?: boolean }) {
  const tool = call.tool
  if (call.plan) return <span className="text-fg-2">Updated the plan · {call.plan.length} steps</span>
  if (proposedPlan(call)) return <span className="text-fg-2">Proposed a plan</span>
  if (!tool || tool.kind === 'other' || !tool.subject) {
    const { name, file } = toolOf(call.text)
    return (
      <span className="min-w-0 font-mono break-all text-fg-1" title={call.text}>
        <span className="font-medium">{tool?.name ?? name}</span>
        <span className="text-fg-2"> {file ?? call.text.slice(name.length + 1, 160)}</span>
      </span>
    )
  }
  const verb = running
    ? (DOING[tool.kind] ?? tool.name)
    : tool.kind === 'edit' && /^write/i.test(tool.name)
      ? 'Wrote'
      : (VERB[tool.kind] ?? tool.name)
  return (
    <span className="min-w-0">
      <span className="text-fg-2">{verb}</span> <span className="font-mono break-all text-fg-1">{tool.subject}</span>
      {tool.kind === 'search' && tool.detail && <span className="text-fg-2"> in {tool.detail}</span>}
    </span>
  )
}

/** How much a result held, as its row says it: `5 lines`, `2 matches`. */
function measure(step: ToolStep): string | null {
  const text = step.result?.text.trim()
  if (!text || !step.call) return null
  const lines = linesOf(step.result!)
  const kind = kindOf(step.call)
  if (kind === 'search') return `${lines} ${lines === 1 ? 'match' : 'matches'}`
  if (kind === 'read') return `${lines} ${lines === 1 ? 'line' : 'lines'}`
  return null
}

function Measure({ step }: { step: ToolStep }) {
  const said = measure(step)
  return said && <span className="shrink-0 text-fg-2">{said}</span>
}

/** A plan the agent put to the person, kept in its turn as it was proposed, folded to its title. */
export function ProposedPlan({ step }: { step: ToolStep }) {
  const plan = proposedPlan(step.call!) ?? ''
  const heading = /^#+\s*(.+)$/m.exec(plan)?.[1]
  return (
    <details className="group/plan rounded-md border border-line bg-elevated text-xs">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-1.5">
        <Icon name="chevronRight" className="h-3 w-3 shrink-0 text-fg-2 transition-transform group-open/plan:rotate-90" />
        <span className="text-fg-2">Proposed a plan</span>
        {heading && <span className="min-w-0 truncate text-fg-1">{heading}</span>}
      </summary>
      <div className="border-t border-line-subtle px-3.5 py-2.5 text-sm">
        <Markdown text={plan} />
      </div>
    </details>
  )
}

/** One step in a run: what it did, with its result, or an edit's diff, a click away. */
export function StepRow({ step, open: opened = false, running = false }: { step: ToolStep; open?: boolean; running?: boolean }) {
  const [open, setOpen] = useState(opened)
  const result = useWhole(step.result, open)
  const plan = step.call ? proposedPlan(step.call) : null
  const change = step.call ? changeOf(step.call) : null
  // A plan's result only tells the model it was saved or approved; the plan itself is what to show.
  const output = result && result.text.trim() && !step.call?.plan && !plan && !change ? result.text : null
  const openable = Boolean(output || plan || change)
  return (
    <div data-testid={change ? 'edit' : undefined} className="flex flex-col gap-1">
      <button
        type="button"
        aria-expanded={openable ? open : undefined}
        disabled={!openable}
        onClick={() => setOpen(!open)}
        className="group/row flex w-full min-w-0 items-start gap-2 py-px text-left max-md:min-h-11"
      >
        <Icon
          name="chevronRight"
          className={cx('mt-0.5 h-3 w-3 shrink-0 text-fg-2 transition-transform', open && 'rotate-90', !openable && 'invisible')}
        />
        {running && !result ? <Spinner className="mt-1" /> : <Mark step={step} />}
        <span className="min-w-0 flex-1 group-enabled/row:group-hover/row:text-accent-fg">
          {step.call ? (
            <Said call={step.call} running={running && !result} />
          ) : (
            <span className="text-fg-2">A tool&apos;s output</span>
          )}
        </span>
        {change ? (
          <span className="shrink-0 font-mono text-2xs">
            <span className="text-ok">+{change.added.length}</span> <span className="text-danger">−{change.removed.length}</span>
          </span>
        ) : (
          <Measure step={step} />
        )}
      </button>
      {open && change && (
        <div className="ml-5">
          <EditCard change={change} bare />
        </div>
      )}
      {open && plan && (
        <div className="ml-5 max-h-96 overflow-auto rounded-sm bg-bg px-3 py-2 text-sm text-fg-1">
          <Markdown text={plan} />
        </div>
      )}
      {open && output && (
        <pre className="ml-5 max-h-64 overflow-auto rounded-sm bg-bg px-2.5 py-2 font-mono text-output whitespace-pre-wrap text-fg-2">
          {output}
        </pre>
      )}
      {step.images.map((image) => (
        <span key={image.index} className="ml-5">
          <ImageThumb entry={image} from="from the tool" />
        </span>
      ))}
    </div>
  )
}

/** How a command ended, as its card says it: `failed · exit 101 · 1 of 2 failed · 3s`, `passed 2 of 2 · 3s`, `1 image`. */
function commandStatus(step: ToolStep): string {
  const outcome = step.result?.outcome
  if (!step.result) return 'running'
  const tests = testsOf(step.result.text)
  const total = tests && tests.passed + tests.failed
  const parts = [
    outcome?.failed ? 'failed' : null,
    outcome?.failed && outcome.exit_code !== null ? `exit ${outcome.exit_code}` : null,
    tests ? (tests.failed > 0 ? `${tests.failed} of ${total} failed` : `passed ${tests.passed} of ${total}`) : null,
    step.call ? duration(step.call.time_ms, step.result.time_ms) : null,
    step.images.length > 0 ? `${step.images.length} ${step.images.length === 1 ? 'image' : 'images'}` : null,
  ].filter(Boolean)
  return parts.length > 0 ? parts.join(' · ') : 'done'
}

/**
 * A command the agent ran, closed to one line: what it was for, as the agent said it, then the command
 * itself, muted, and how it ended. Opened, its output's last lines and any pictures it made. `card` draws
 * it on its own, as a failed command stands; in a run it is one of the run's rows.
 */
export function CommandCard({ step, card = false, running = false }: { step: ToolStep; card?: boolean; running?: boolean }) {
  const [all, setAll] = useState(false)
  const call = step.call!
  const outcome = step.result?.outcome
  // Under way, its output so far, as the agent streams it; once it ends, what was recorded, whole once all is asked for.
  const live = useContext(LiveTool)
  const under = running && !step.result
  const result = useWhole(step.result, all)
  const output = result?.text ?? (under && live.id !== null && live.id === call.tool?.id ? live.text : '')
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const [open, setOpen] = useState(verbose)
  const shortened = useMemo(() => tailOf(output), [output])
  // What its page left out counts among the lines a click shows.
  const tail = result?.cut ? { ...shortened, hidden: result.cut.lines - shortened.lines.length } : shortened
  const command = call.tool?.subject ?? toolOf(call.text).name
  const action = call.tool?.detail?.trim()
  const waiting = useContext(LiveQuestions)?.awaiting.includes(call.tool?.id ?? '') ?? false
  const openable = under || Boolean(output.trim()) || step.images.length > 0 || command.length > 80 || command.includes('\n')
  return (
    <div data-testid="command" className={cx('text-xs', card && 'overflow-hidden rounded-md border border-line bg-elevated')}>
      <button
        type="button"
        aria-expanded={openable ? open : undefined}
        disabled={!openable}
        onClick={() => setOpen(!open)}
        title={command}
        className={cx(
          'group/row flex w-full min-w-0 items-center gap-2 text-left max-md:min-h-11',
          card ? 'px-3 py-1.5' : 'py-px',
          open && card && 'border-b border-line-subtle',
        )}
      >
        <Icon
          name="chevronRight"
          className={cx('h-3 w-3 shrink-0 text-fg-2 transition-transform', open && 'rotate-90', !openable && 'invisible')}
        />
        {running && !step.result ? <Spinner /> : <Mark step={step} />}
        <span
          className={cx(
            'min-w-0 shrink truncate group-enabled/row:group-hover/row:text-accent-fg',
            outcome?.failed ? 'text-danger' : 'text-fg-1',
            running && 'font-medium',
          )}
        >
          {action ?? <span className="font-mono">{shortCommand(command.trim().split('\n')[0]!)}</span>}
        </span>
        {action && (
          <span className="min-w-0 flex-1 truncate font-mono text-output text-fg-3 max-sm:hidden">
            {shortCommand(command.trim().split('\n')[0]!)}
          </span>
        )}
        <span
          className={cx(
            'ml-auto min-w-0 shrink truncate tabular-nums',
            outcome?.failed ? 'text-danger' : waiting ? 'text-warn-fg' : 'text-fg-2',
          )}
        >
          {waiting ? 'waiting for you' : under ? <Running since={call.time_ms} /> : commandStatus(step)}
        </span>
      </button>
      {open && (
        <div className={cx('flex flex-col gap-1.5', card ? 'px-3 py-2' : 'mt-1 ml-5 rounded-sm bg-bg px-2.5 py-2')}>
          <pre className="font-mono text-output whitespace-pre-wrap text-fg-1 [overflow-wrap:anywhere]">
            <span className="text-fg-2">$ </span>
            {command}
          </pre>
          {output.trim() && (
            <pre
              data-testid={under ? 'tool-output' : undefined}
              className="max-h-80 overflow-auto font-mono text-output leading-relaxed whitespace-pre-wrap text-fg-2"
            >
              {(all && !result?.cut ? output.replace(/\s+$/, '').split('\n') : tail.lines).map((line, index) => (
                <span key={index} className={cx('block', /\b(FAILED|error|panicked)\b/.test(line) && 'bg-danger-tint text-fg-1')}>
                  {line || ' '}
                </span>
              ))}
            </pre>
          )}
          {tail.hidden > 0 && (
            <button
              type="button"
              onClick={() => setAll(!all)}
              className="w-fit text-2xs text-accent-fg hover:text-fg-1 max-md:min-h-11"
            >
              {all ? 'Show the last lines' : `Show all ${tail.hidden + tail.lines.length} lines`}
            </button>
          )}
          {step.images.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {step.images.map((image) => (
                <ImageThumb key={image.index} entry={image} from="from the command" />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** A command under way, and how long it has run; its output is a click away. */
function Running({ since }: { since: number }) {
  const now = useNow(1000)
  const seconds = Math.max(0, Math.floor((now - since) / 1000))
  return (
    <span>
      running{' '}
      <span data-volatile>
        · {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, '0')}
      </span>
    </span>
  )
}

/** A question the agent waits on, answered in place: a digit picks a choice, Esc skips it. */
export function AnswerForm({ thread, asked }: { thread: string; asked: QuestionAsked }) {
  const [chosen, setChosen] = useState<Record<string, string[]>>({})
  const [error, setError] = useState<string>()
  const [sending, setSending] = useState(false)
  const send = (answers: Record<string, string[]> | null) => {
    setError(undefined)
    setSending(true)
    api.answerQuestion(thread, asked.id, answers).catch((failure: unknown) => {
      setSending(false)
      setError(message(failure))
    })
  }
  const single = asked.fields.length === 1 && !asked.fields[0]!.multi && asked.fields[0]!.options.length > 0
  const pick = (key: string, value: string, multi: boolean) => {
    if (single) return send({ [key]: [value] })
    const had = chosen[key] ?? []
    setChosen({
      ...chosen,
      [key]: multi ? (had.includes(value) ? had.filter((each) => each !== value) : [...had, value]) : [value],
    })
  }
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (sending || event.metaKey || event.ctrlKey || event.altKey || document.querySelector('dialog[open]')) return
      if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return
      const field = asked.fields[0]
      const index = Number(event.key) - 1
      if (field && Number.isInteger(index) && index >= 0 && index < Math.min(field.options.length, 9)) {
        event.preventDefault()
        pick(field.key, field.options[index]!.value, field.multi)
      } else if (event.key === 'Escape') {
        event.preventDefault()
        send(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })
  const filled = (field: QuestionAsked['fields'][number]) => (chosen[field.key] ?? []).some((value) => value.trim())
  // Claude's adapter follows each question with an optional box for an answer of your own, and marks nothing required.
  const optional = (field: QuestionAsked['fields'][number]) => !field.required && field.options.length === 0
  const questions = asked.fields.filter((field) => !optional(field))
  const complete = asked.fields.every((field) => !field.required || filled(field)) && asked.fields.some(filled)
  const answers = () => Object.fromEntries(asked.fields.filter(filled).map((field) => [field.key, chosen[field.key]!]))
  const [own, setOwn] = useState('')
  // A form's choices take no free text, so your own answer steers the agent: the daemon closes the question with it and prompts with it.
  const answerOwn = () => {
    setError(undefined)
    setSending(true)
    api.steer(thread, own.trim()).catch((failure: unknown) => {
      setSending(false)
      setError(message(failure))
    })
  }
  const choosing = asked.fields.some((field) => field.options.length > 0)
  return (
    <div className="flex flex-col gap-3">
      {asked.fields.map((field, fieldIndex) => (
        <div key={field.key} className="flex flex-col gap-1.5">
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{field.title}</span>
            <span className={cx(optional(field) ? 'text-fg-2' : 'text-sm font-medium')}>
              {questions.length === 1 && !optional(field) ? asked.message : (field.description ?? field.title)}
            </span>
            {optional(field) ? (
              <span className="ml-auto shrink-0 text-fg-2">optional</span>
            ) : (
              <span className="ml-auto shrink-0 text-warn-fg">
                {questions.length > 1 && (
                  <span className="mr-2 font-mono text-fg-2">
                    {questions.indexOf(field) + 1} of {questions.length}
                  </span>
                )}
                waiting on you
              </span>
            )}
          </div>
          {field.options.length === 0 ? (
            <input
              aria-label={field.title}
              className="rounded-sm border border-line bg-bg px-2.5 py-1.5 text-xs focus:border-accent focus:outline-none"
              placeholder="Your answer"
              value={chosen[field.key]?.[0] ?? ''}
              onChange={(event) => setChosen({ ...chosen, [field.key]: [event.target.value] })}
            />
          ) : (
            <ul className="flex flex-col gap-0.5">
              {field.options.map(({ value, label, description }, index) => {
                const picked = (chosen[field.key] ?? []).includes(value)
                return (
                  <li key={value}>
                    <button
                      type="button"
                      role={field.multi ? 'checkbox' : 'radio'}
                      aria-checked={picked}
                      disabled={sending}
                      onClick={() => pick(field.key, value, field.multi)}
                      className={cx(
                        'flex w-full items-start gap-2 rounded-sm border px-2 py-1.5 text-left max-md:min-h-11',
                        picked
                          ? 'border-accent bg-accent-tint'
                          : 'border-line-subtle hover:border-line-strong hover:bg-surface-1',
                      )}
                    >
                      {fieldIndex === 0 && index < 9 ? <Kbd>{index + 1}</Kbd> : <span className="w-4 shrink-0" />}
                      <span className="flex-1">
                        <span className={cx('text-fg-1', picked && 'font-medium')}>{label}</span>
                        {description && <span className="block text-fg-2">{description}</span>}
                      </span>
                      {picked && <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent-fg" />}
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      ))}
      {choosing && (
        <form
          className="flex items-center gap-2 rounded-sm border border-dashed border-line px-2 py-1"
          onSubmit={(event) => {
            event.preventDefault()
            if (own.trim()) answerOwn()
          }}
        >
          <Icon name="edit" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
          <input
            aria-label="something else"
            className="min-w-0 flex-1 bg-transparent py-0.5 text-xs placeholder:text-fg-2 focus:outline-none"
            placeholder="Something else: type your own answer"
            value={own}
            disabled={sending}
            onChange={(event) => setOwn(event.target.value)}
          />
          {own.trim() && (
            <Button size="sm" variant="primary" type="submit" disabled={sending}>
              Answer
            </Button>
          )}
        </form>
      )}
      <span className="flex flex-wrap items-center gap-1.5">
        {!single && (
          <Button variant="primary" disabled={!complete || sending} onClick={() => send(answers())}>
            Answer
          </Button>
        )}
        <Button variant="ghost" disabled={sending} onClick={() => send(null)}>
          Skip <Kbd>Esc</Kbd>
        </Button>
        {asked.fields[0]!.options.length > 1 && (
          <span className="text-fg-2 pointer-coarse:hidden">1–{Math.min(asked.fields[0]!.options.length, 9)} to pick</span>
        )}
        {error && <span className="text-danger">{error}</span>}
      </span>
    </div>
  )
}

/** A question the agent asked, with the choices it offered and the one taken; while the agent waits on it, the form to answer it. */
export function QuestionCard({ step }: { step: ToolStep }) {
  const live = useContext(LiveQuestions)
  const verbose = useContext(TranscriptDetail) === 'verbose'
  // Once answered, the answer is what matters; the choices are a click away, or open in Verbose.
  const [showing, setShowing] = useState(verbose)
  const questions: QuestionView[] = step.call?.tool?.questions ?? []
  const answers = step.result?.outcome?.answers ?? []
  const asked = !step.result && live?.questions.find((question) => step.call?.tool?.id === `question-${question.id}`)
  if (asked && live)
    return (
      <section
        aria-label="question"
        data-testid="question"
        className="flex flex-col gap-3 rounded-md border border-warn bg-elevated px-3 py-2.5 text-xs"
      >
        <span className="flex items-center gap-2 text-fg-2">
          <span className="h-1.5 w-1.5 rounded-full bg-warn" />
          {live.agent} asks
        </span>
        <AnswerForm thread={live.thread} asked={asked} />
      </section>
    )
  if (questions.length === 0) return <StepRow step={step} />
  if (step.result && !showing)
    return (
      <section
        aria-label="question"
        data-testid="question"
        className="flex flex-col gap-1 rounded-md border border-line bg-elevated px-3 py-2 text-xs"
      >
        {questions.map((question) => {
          const answer = answers.find((each) => each.question === question.question)?.answer
          return (
            <div key={question.question} className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
              {question.header && (
                <span className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{question.header}</span>
              )}
              <span className="font-medium">{question.question}</span>
              <span className="ml-auto shrink-0 text-fg-2">{answer !== undefined ? 'answered by you' : 'not answered'}</span>
              {answer !== undefined && (
                <span className="flex w-full items-center gap-1.5">
                  <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-ok" />
                  <span className="text-fg-1">{answer}</span>
                </span>
              )}
            </div>
          )
        })}
        <button
          type="button"
          onClick={() => setShowing(true)}
          className="w-fit text-2xs text-accent-fg hover:text-fg-1 max-md:min-h-11"
        >
          Show the choices
        </button>
      </section>
    )
  return (
    <section
      aria-label="question"
      data-testid="question"
      className="flex flex-col gap-3 rounded-md border border-line bg-elevated px-3 py-2.5 text-xs"
    >
      {questions.map((question) => {
        const answer = answers.find((each) => each.question === question.question)?.answer
        const chosen = answer?.split(', ') ?? []
        return (
          <div key={question.question} className="flex flex-col gap-1.5">
            <div className="flex items-baseline gap-2">
              {question.header && (
                <span className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{question.header}</span>
              )}
              <span className="text-sm font-medium">{question.question}</span>
              <span className="ml-auto shrink-0 text-fg-2">
                {answer !== undefined ? 'answered by you' : step.result ? 'not answered' : 'waiting'}
              </span>
            </div>
            <ul className="flex flex-col gap-0.5">
              {question.options.map((option) => {
                const picked = chosen.includes(option.label)
                return (
                  <li
                    key={option.label}
                    className={cx(
                      'flex items-start gap-2 rounded-sm px-2 py-1.5',
                      picked ? 'bg-accent-tint shadow-[inset_2px_0_0_var(--accent)]' : 'text-fg-2',
                    )}
                  >
                    {picked ? (
                      <Icon name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok" />
                    ) : (
                      <span className="w-3.5 shrink-0" />
                    )}
                    <span>
                      <span className={cx(picked && 'font-medium text-fg-1')}>{option.label}</span>
                      {option.description && <span className="block text-fg-2">{option.description}</span>}
                    </span>
                  </li>
                )
              })}
              {answer !== undefined && !question.options.some((option) => chosen.includes(option.label)) && (
                <li className="flex items-start gap-2 rounded-sm bg-accent-tint px-2 py-1.5 shadow-[inset_2px_0_0_var(--accent)]">
                  <Icon name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok" />
                  <span className="font-medium">{answer}</span>
                </li>
              )}
            </ul>
          </div>
        )
      })}
    </section>
  )
}

const STEP_MARK: Record<string, string> = { completed: 'done', in_progress: 'doing now', pending: 'to do' }

/** The agent's plan: each step, ticked when done and marked while under way. */
export function PlanSteps({ entry }: { entry: HistoryEntry }) {
  const steps = entry.plan ?? []
  const done = steps.filter((step) => step.status === 'completed').length
  return (
    <section
      aria-label="plan steps"
      data-testid="plan-steps"
      className="flex flex-col gap-1.5 rounded-md border border-line bg-elevated px-3 py-2.5 text-xs"
    >
      <span className="flex items-center gap-2 text-fg-2">
        <Icon name="list" className="h-3.5 w-3.5" />
        <span className="font-medium text-fg-1">Plan</span>
        <span>
          {done} of {steps.length} done
        </span>
      </span>
      <ol className="flex flex-col gap-1">
        {steps.map((step, index) => (
          <li key={index} className="flex items-start gap-2">
            {step.status === 'completed' ? (
              <Icon name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok" />
            ) : step.status === 'in_progress' ? (
              <span className="mt-0.5 h-3.5 w-3.5 shrink-0 rounded-full border-2 border-accent bg-accent-tint" />
            ) : (
              <span className="mt-0.5 h-3.5 w-3.5 shrink-0 rounded-full border border-line-strong" />
            )}
            <span
              className={cx(
                step.status === 'completed'
                  ? 'text-fg-2 line-through decoration-fg-3'
                  : step.status === 'in_progress'
                    ? 'font-medium text-fg-1'
                    : 'text-fg-1',
              )}
            >
              {step.text}
              <span className="sr-only"> ({STEP_MARK[step.status] ?? step.status})</span>
            </span>
          </li>
        ))}
      </ol>
    </section>
  )
}
