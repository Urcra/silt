import { useContext, useEffect, useState } from 'react'
import { api, type HistoryEntry, type QuestionAsked, type QuestionView } from '../api'
import { LiveQuestions } from '../liveQuestions'
import { message } from '../query'
import { kindOf, tailOf, toolOf, type ToolStep } from '../transcript'
import { Button, Dialog, Icon, Kbd, cx } from '../ui'

/** A picture a prompt or a tool carried, small, opening at full size. */
export function ImageThumb({ entry, from }: { entry: HistoryEntry; from: string }) {
  const [open, setOpen] = useState(false)
  const image = entry.image
  if (!image) return null
  if (!image.data) return <span className="text-2xs text-fg-2">An image too large to show here ({image.media_type})</span>
  const src = `data:${image.media_type};base64,${image.data}`
  return (
    <>
      <button
        type="button"
        aria-label={`open the image ${from}`}
        onClick={() => setOpen(true)}
        className="w-fit overflow-hidden rounded-md border border-line bg-surface-1 hover:border-line-strong"
      >
        <img src={src} alt={`An image ${from}`} className="block max-h-44 max-w-[320px] object-contain" />
      </button>
      <Dialog open={open} onClose={() => setOpen(false)} title={`An image ${from}`} wide>
        <img src={src} alt={`An image ${from}`} className="max-h-[75vh] w-full object-contain" />
      </Dialog>
    </>
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

const VERB: Record<string, string> = { read: 'Read', search: 'Searched', edit: 'Edited', execute: 'Ran', question: 'Asked', fetch: 'Fetched' }

/** What a step did, in its tool's words: `Read src/upload.rs`, `Searched progress in src`. */
function Said({ call }: { call: HistoryEntry }) {
  const tool = call.tool
  if (call.plan) return <span className="text-fg-2">Updated the plan · {call.plan.length} steps</span>
  if (!tool || tool.kind === 'other' || !tool.subject) {
    const { name, file } = toolOf(call.text)
    return (
      <span className="min-w-0 font-mono break-all text-fg-1" title={call.text}>
        <span className="font-medium">{tool?.name ?? name}</span>
        <span className="text-fg-2"> {file ?? call.text.slice(name.length + 1, 160)}</span>
      </span>
    )
  }
  const verb = tool.kind === 'edit' && /^write/i.test(tool.name) ? 'Wrote' : (VERB[tool.kind] ?? tool.name)
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
  const lines = text.split('\n').length
  const kind = kindOf(step.call)
  if (kind === 'search') return `${lines} ${lines === 1 ? 'match' : 'matches'}`
  if (kind === 'read') return `${lines} ${lines === 1 ? 'line' : 'lines'}`
  return null
}

/** One step in a fold: what it did, with its result a click away. */
export function StepRow({ step, open: opened = false }: { step: ToolStep; open?: boolean }) {
  const [open, setOpen] = useState(opened)
  const result = step.result
  // A plan's result only tells the model it was saved; its steps show on the plan.
  const output = result && result.text.trim() && !step.call?.plan ? result.text : null
  return (
    <div className="flex flex-col gap-1">
      <span className="flex items-start gap-2">
        <Mark step={step} />
        <span className="min-w-0 flex-1">{step.call ? <Said call={step.call} /> : <span className="text-fg-2">A tool&apos;s output</span>}</span>
        {measure(step) && <span className="shrink-0 text-fg-2">{measure(step)}</span>}
        {output && (
          <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="shrink-0 text-fg-2 hover:text-fg-1 max-md:min-h-11">
            {open ? 'Hide output' : 'Output'}
          </button>
        )}
      </span>
      {open && output && (
        <pre className="ml-[22px] max-h-64 overflow-auto rounded-sm bg-bg px-2.5 py-2 font-mono text-[11.5px] whitespace-pre-wrap text-fg-2">{output}</pre>
      )}
      {step.images.map((image) => (
        <span key={image.index} className="ml-[22px]">
          <ImageThumb entry={image} from="from the tool" />
        </span>
      ))}
    </div>
  )
}

/** A command the agent ran: the command, how it ended, its output's last lines and any pictures it made. */
export function CommandCard({ step }: { step: ToolStep }) {
  const [all, setAll] = useState(false)
  const call = step.call!
  const outcome = step.result?.outcome
  const output = step.result?.text ?? ''
  const tail = tailOf(output)
  const command = call.tool?.subject ?? toolOf(call.text).name
  return (
    <div data-testid="command" className="overflow-hidden rounded-md border border-line bg-elevated text-xs">
      <div className={cx('flex items-center gap-2 px-3 py-1.5', output.trim() && 'border-b border-line-subtle')}>
        <Mark step={step} />
        <span className="min-w-0 flex-1 font-mono break-words">
          <span className="text-fg-2">$ </span>
          {command}
        </span>
        <span className={cx('shrink-0', outcome?.failed ? 'text-danger' : 'text-fg-2')}>
          {outcome?.failed ? ['failed', outcome.exit_code !== null && `exit ${outcome.exit_code}`].filter(Boolean).join(' · ') : step.result ? 'done' : 'running'}
        </span>
      </div>
      {call.tool?.detail && <p className="px-3 pt-1.5 text-fg-2">{call.tool.detail}</p>}
      {output.trim() && (
        <div className="flex flex-col gap-1 px-3 py-2">
          <pre className="max-h-80 overflow-auto font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-fg-2">
            {(all ? output.replace(/\s+$/, '').split('\n') : tail.lines).map((line, index) => (
              <span key={index} className={cx('block', /\b(FAILED|error|panicked)\b/.test(line) && 'bg-danger-tint text-fg-1')}>
                {line || ' '}
              </span>
            ))}
          </pre>
          {tail.hidden > 0 && (
            <button type="button" onClick={() => setAll(!all)} className="w-fit text-2xs text-accent-fg hover:text-fg-1 max-md:min-h-11">
              {all ? 'Show the last lines' : `Show all ${tail.hidden + tail.lines.length} lines`}
            </button>
          )}
        </div>
      )}
      {step.images.length > 0 && (
        <div className="flex flex-wrap gap-2 px-3 pb-2.5">
          {step.images.map((image) => (
            <ImageThumb key={image.index} entry={image} from="from the command" />
          ))}
        </div>
      )}
    </div>
  )
}

/** A question the agent waits on, answered in place: a digit picks a choice, Esc skips it. */
function AnswerForm({ thread, asked }: { thread: string; asked: QuestionAsked }) {
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
    setChosen({ ...chosen, [key]: multi ? (had.includes(value) ? had.filter((each) => each !== value) : [...had, value]) : [value] })
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
  const complete = asked.fields.every((field) => (chosen[field.key] ?? []).some((value) => value.trim()))
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
            <span className="text-sm font-medium">{asked.fields.length === 1 ? asked.message : (field.description ?? field.title)}</span>
            <span className="ml-auto shrink-0 text-warn">
              {asked.fields.length > 1 && <span className="mr-2 font-mono text-fg-2">{fieldIndex + 1} of {asked.fields.length}</span>}
              waiting on you
            </span>
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
                        picked ? 'border-accent bg-accent-tint' : 'border-line-subtle hover:border-line-strong hover:bg-surface-1',
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
          <Button variant="primary" disabled={!complete || sending} onClick={() => send(chosen)}>
            Answer
          </Button>
        )}
        <Button variant="ghost" disabled={sending} onClick={() => send(null)}>
          Skip <Kbd>Esc</Kbd>
        </Button>
        {asked.fields[0]!.options.length > 1 && (
          <span className="text-fg-2 pointer-coarse:hidden">
            1–{Math.min(asked.fields[0]!.options.length, 9)} to pick
          </span>
        )}
        {error && <span className="text-danger">{error}</span>}
      </span>
    </div>
  )
}

/** A question the agent asked, with the choices it offered and the one taken; while the agent waits on it, the form to answer it. */
export function QuestionCard({ step }: { step: ToolStep }) {
  const live = useContext(LiveQuestions)
  const questions: QuestionView[] = step.call?.tool?.questions ?? []
  const answers = step.result?.outcome?.answers ?? []
  const asked = !step.result && live?.questions.find((question) => step.call?.tool?.id === `question-${question.id}`)
  if (asked && live)
    return (
      <section aria-label="question" data-testid="question" className="flex flex-col gap-3 rounded-md border border-warn bg-elevated px-3 py-2.5 text-xs">
        <AnswerForm thread={live.thread} asked={asked} />
      </section>
    )
  if (questions.length === 0) return <StepRow step={step} />
  return (
    <section aria-label="question" data-testid="question" className="flex flex-col gap-3 rounded-md border border-line bg-elevated px-3 py-2.5 text-xs">
      {questions.map((question) => {
        const answer = answers.find((each) => each.question === question.question)?.answer
        const chosen = answer?.split(', ') ?? []
        return (
          <div key={question.question} className="flex flex-col gap-1.5">
            <div className="flex items-baseline gap-2">
              {question.header && <span className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{question.header}</span>}
              <span className="text-sm font-medium">{question.question}</span>
              <span className="ml-auto shrink-0 text-fg-2">{answer !== undefined ? 'answered' : step.result ? 'not answered' : 'waiting'}</span>
            </div>
            <ul className="flex flex-col gap-0.5">
              {question.options.map((option) => {
                const picked = chosen.includes(option.label)
                return (
                  <li
                    key={option.label}
                    className={cx('flex items-start gap-2 rounded-sm px-2 py-1.5', picked ? 'bg-accent-tint shadow-[inset_2px_0_0_var(--accent)]' : 'text-fg-2')}
                  >
                    {picked ? <Icon name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok" /> : <span className="w-3.5 shrink-0" />}
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
    <section aria-label="plan steps" data-testid="plan-steps" className="flex flex-col gap-1.5 rounded-md border border-line bg-elevated px-3 py-2.5 text-xs">
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
            <span className={cx(step.status === 'completed' ? 'text-fg-2 line-through decoration-fg-3' : step.status === 'in_progress' ? 'font-medium text-fg-1' : 'text-fg-1')}>
              {step.text}
              <span className="sr-only"> ({STEP_MARK[step.status] ?? step.status})</span>
            </span>
          </li>
        ))}
      </ol>
    </section>
  )
}
