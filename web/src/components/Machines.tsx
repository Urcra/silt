import { useQuery } from '@tanstack/react-query'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { api } from '../api'
import {
  type Backend,
  MACHINE_COLOURS,
  type MachineColour,
  type MachineFields,
  type Mark,
  type MachineTest,
  type SwitchFailure,
  type TestStep,
  addMachine,
  changeMachine,
  disconnectMachine,
  installSilt,
  removeMachine,
  setMachineColour,
  setMachinesList,
  setMark,
  switchBackend,
  testMachine,
  useBackends,
  useMachineTest,
  useMachinesView,
  useSwitchFailed,
} from '../desktop'
import { colourOf, machineName } from '../elsewhere'
import { useNow } from '../now'
import { keys } from '../query'
import { fieldsOf, machineProblem, withCode } from '../machines'
import { SETTINGS_HEADING } from '../settingsContext'
import { ago } from '../time'
import { Banner, Button, Chip, Dot, Icon, type IconName, Input, Menu, Popover, Spinner, cx, toast } from '../ui'
import { MachineChip, MachineInitial } from './MachineMark'

/** Says why switching machines failed; mounted once, beside the toasts. */
export function SwitchFailures() {
  useSwitchFailed(useCallback((failure: SwitchFailure) => toast(<>{withCode(failure.reason)}</>, 'danger'), []))
  return null
}

const since = (ms: number, now: number) => {
  const said = ago(ms, now)
  return /^\d/.test(said) ? `${said} ago` : said
}

function Status({ backend, shown, installing }: { backend: Backend; shown: boolean; installing?: string }) {
  if (installing) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs text-fg-2">
        <Spinner />
        {installing}
      </span>
    )
  }
  if (shown) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs">
        <Dot tone="accent" />
        shown here
      </span>
    )
  }
  // This computer and WSL have nothing to connect to.
  if (backend.kind !== 'ssh') return null
  switch (backend.state) {
    case 'connecting':
      return (
        <span className="inline-flex items-center gap-1.5 text-xs text-fg-2">
          <Spinner />
          connecting…
        </span>
      )
    case 'connected':
      return (
        <span className="inline-flex items-center gap-1.5 text-xs">
          <Dot tone="ok" />
          connected
        </span>
      )
    case 'failed':
      return (
        <span className="inline-flex items-center gap-1.5 text-xs text-danger">
          <Dot tone="danger" />
          couldn’t connect
        </span>
      )
    default:
      return (
        <span className="inline-flex items-center gap-1.5 text-xs text-fg-2">
          <Dot tone="idle" />
          not connected
        </span>
      )
  }
}

function Row({
  name,
  icon,
  destination,
  port,
  status,
  meta,
  actions,
  children,
  colour,
}: {
  name: string
  icon: IconName
  destination?: string
  port?: number | null
  status: ReactNode
  meta: ReactNode
  actions?: ReactNode
  children?: ReactNode
  /** Its colour's picker, in the desktop app (ADR 0136). */
  colour?: ReactNode
}) {
  return (
    <section
      aria-label={name}
      className="grid grid-cols-[20px_minmax(0,1fr)] gap-x-3 border-b border-line-subtle px-4 py-3.5 last:border-b-0"
    >
      <Icon name={icon} className="mt-0.5 h-5 w-5 text-fg-2" />
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">{name}</span>
          {destination && <code className="font-mono text-xs text-fg-2">{destination}</code>}
          {port && (
            <span className="inline-flex items-center rounded-sm bg-surface-2 px-1.5 font-mono text-label text-fg-2">
              port {port}
            </span>
          )}
          <span className="ml-auto inline-flex items-center gap-2">
            {colour}
            {status}
            {actions}
          </span>
        </div>
        <p className="text-xs text-fg-2">{meta}</p>
        {children}
      </div>
    </section>
  )
}

function joined(parts: ReactNode[]): ReactNode {
  return parts.filter(Boolean).flatMap((part, at) => (at === 0 ? [part] : [' · ', part]))
}

const restarts = (agents: number) =>
  agents === 1
    ? 'its daemon restarts on it once its agent finishes'
    : `its daemon restarts on it once its ${agents} agents finish`

function MachineRow({
  machine,
  shown,
  editable,
  latest,
  onEdit,
}: {
  machine: Backend
  shown: boolean
  editable: boolean
  /** This computer's build, which the machine is offered to update to. */
  latest?: string | null
  onEdit: () => void
}) {
  const now = useNow()
  const [report, setReport] = useMachineTest()
  const fields: MachineFields = { name: machine.label, destination: machine.destination ?? '', port: machine.port }
  const alias = machine.destination && !machine.destination.includes('@') && !machine.destination.includes('.')
  const install = report?.action === 'install' && report.name === machine.label ? report : undefined
  const verb = machine.canInstall ? 'installing' : 'updating'
  // An update that failed before it changed anything leaves the machine connected, on its old build.
  const updateFailed = !machine.installing && install?.done && !install.ok && machine.state !== 'failed'
  const failedStep = install?.steps.find((step) => step.state === 'failed')
  const offered =
    !machine.installing &&
    !updateFailed &&
    !!latest &&
    !!machine.version &&
    machine.version !== latest &&
    (machine.state === 'connected' || shown)
  const meta =
    machine.state === 'connecting'
      ? machine.step
      : machine.state === 'failed'
        ? machine.since && `Tried ${since(machine.since, now)}`
        : machine.pendingRestart && machine.state === 'connected'
          ? joined([machine.version && `silt ${machine.version} installed`, restarts(machine.pendingRestart)])
          : machine.state === 'connected' || shown
            ? joined([
                machine.version && `silt ${machine.version}`,
                !shown && machine.since && `connected ${since(machine.since, now)}`,
              ])
            : joined([
                machine.lastShown ? `Last shown ${since(machine.lastShown, now)}` : 'Not shown yet',
                alias && (
                  <>
                    a <code className="font-mono">Host</code> in <code className="font-mono">~/.ssh/config</code>
                  </>
                ),
              ])
  const live = machine.state === 'connected' || machine.state === 'connecting'
  return (
    <Row
      name={machine.label}
      icon="terminal"
      destination={machine.destination}
      port={machine.port}
      status={<Status backend={machine} shown={shown} installing={machine.installing ? `${verb}…` : undefined} />}
      colour={<ColourPicker backend={machine} />}
      meta={meta}
      actions={
        <>
          {!shown && !machine.installing && machine.state !== 'failed' && machine.state !== 'connecting' && (
            <Button size="sm" onClick={() => switchBackend(machine.id)}>
              Open
            </Button>
          )}
          {editable && (
            <Menu
              label={`${machine.label}’s options`}
              trigger={<Icon name="more" />}
              items={[
                { label: 'Edit…', onSelect: onEdit },
                { label: 'Test the connection', onSelect: () => testMachine(fields, false) },
                ...(offered ? [{ label: 'Update Silt there', onSelect: () => installSilt(fields, false) }] : []),
                { label: 'Disconnect', onSelect: () => disconnectMachine(machine.label), disabled: !live },
                { label: 'Remove from this list', onSelect: () => removeMachine(machine.label), danger: true },
              ]}
            />
          )}
        </>
      }
    >
      {machine.installing && install && !install.done && <Steps test={install} label={verb} />}
      {offered && (
        <div className="flex flex-wrap items-center gap-2 text-xs text-fg-2">
          {editable ? (
            <>
              <span>This computer has silt {latest}.</span>
              <Button size="sm" icon="arrowUp" onClick={() => installSilt(fields, false)}>
                Update {machine.label}
              </Button>
            </>
          ) : (
            <span>
              This computer has silt {latest}. Update {machine.label} from This computer.
            </span>
          )}
        </div>
      )}
      {updateFailed && (
        <>
          <Banner tone="danger" urgent>
            {withCode(
              `Couldn’t update \`${machine.label}\`. ${failedStep?.text ?? ''} \`${machine.label}\` still runs silt ${machine.version}.`,
            )}
          </Banner>
          <div className="flex flex-wrap gap-2">
            {editable && (
              <Button size="sm" onClick={() => installSilt(fields, false)}>
                Try again
              </Button>
            )}
            <Button size="sm" variant="ghost" onClick={() => setReport(undefined)}>
              Dismiss
            </Button>
          </div>
        </>
      )}
      {machine.state === 'failed' && machine.reason && !machine.installing && (
        <>
          <Banner tone="danger" urgent>
            {withCode(machine.reason)}
          </Banner>
          <div className="flex flex-wrap gap-2">
            {editable && machine.canInstall && (
              <Button size="sm" variant="primary" icon="download" onClick={() => installSilt(fields, false)}>
                Install Silt there
              </Button>
            )}
            {/* A machine's own page can't edit the list, but it can still switch to another. */}
            <Button size="sm" onClick={() => (editable ? testMachine(fields, false) : switchBackend(machine.id))}>
              Retry
            </Button>
            {editable && (
              <Button size="sm" variant="ghost" onClick={onEdit}>
                Edit
              </Button>
            )}
          </div>
        </>
      )}
    </Row>
  )
}

function StepMark({ state }: { state: TestStep['state'] }) {
  return (
    <span className="inline-flex h-[18px] w-3.5 shrink-0 items-center justify-center">
      {state === 'done' ? (
        <Icon name="check" className="h-3.5 w-3.5 text-ok" />
      ) : state === 'running' ? (
        <Spinner />
      ) : state === 'failed' ? (
        <Icon name="x" className="h-3.5 w-3.5 text-danger" />
      ) : (
        <Dot tone="idle" />
      )}
    </span>
  )
}

function Steps({ test, label = 'testing the connection' }: { test: MachineTest; label?: string }) {
  return (
    <ol aria-label={label} className="flex flex-col gap-1.5 rounded-md border border-line-subtle bg-surface-1 px-3 py-2.5">
      {test.steps
        .filter((step) => !test.done || step.state !== 'waiting')
        .map((step) => (
          <li
            key={step.text}
            className={cx(
              'flex items-start gap-2 text-xs leading-normal',
              step.state === 'waiting' && 'text-fg-2',
              step.state === 'failed' && 'text-danger',
            )}
          >
            <StepMark state={step.state} />
            <span className="min-w-0 break-words">{withCode(step.text)}</span>
          </li>
        ))}
    </ol>
  )
}

function Field({ id, label, hint, children }: { id: string; label: string; hint: ReactNode; children: ReactNode }) {
  return (
    <>
      <label htmlFor={id} className="text-xs text-fg-2">
        {label}
      </label>
      <div className="flex flex-wrap items-center gap-2">
        {children}
        <span className="text-2xs text-fg-2">{hint}</span>
      </div>
    </>
  )
}

/** Adding a machine tests it first, step by step; editing one only saves it. */
function MachineForm({ editing, taken, onDone }: { editing?: Backend; taken: string[]; onDone: () => void }) {
  const [form, setForm] = useState({
    name: editing?.label ?? '',
    destination: editing?.destination ?? '',
    port: editing?.port ? String(editing.port) : '',
  })
  const [touched, setTouched] = useState(false)
  const [testing, setTesting] = useState(false)
  const [action, setAction] = useState<'test' | 'install'>('test')
  const [report, setReport] = useMachineTest()
  const test = testing && report?.name === form.name && (report.action ?? 'test') === action ? report : undefined
  const added = test?.done && test.ok
  useEffect(() => {
    // The desktop app adds it once the test passes.
    if (added) onDone()
  }, [added, onDone])
  const problem = machineProblem(form, taken)
  const running = testing && !test?.done
  const failed = test?.done && !test.ok
  const set = (field: keyof typeof form) => (event: { target: { value: string } }) => {
    setForm({ ...form, [field]: event.target.value })
    setTesting(false)
    // Only the Install button installs: Enter after an edit tests again.
    setAction('test')
  }
  const start = (next: 'test' | 'install' = 'test') => {
    setTouched(true)
    if (problem) return
    setReport(undefined)
    setTesting(true)
    setAction(next)
    if (next === 'install') installSilt(fieldsOf(form), true)
    else testMachine(fieldsOf(form), true)
  }
  const save = () => {
    setTouched(true)
    if (problem || !editing) return
    changeMachine(editing.label, fieldsOf(form))
    onDone()
  }
  const title = editing ? `Edit ${editing.label}` : 'Add a machine'
  return (
    <section aria-label={title} className="flex flex-col gap-3 rounded-md border border-line bg-elevated p-4">
      <h2 className="text-sm font-medium">{title}</h2>
      <form
        className="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-x-3 gap-y-2.5"
        onSubmit={(event) => {
          event.preventDefault()
          if (editing) save()
          else start(action)
        }}
      >
        <Field id="machine-name" label="Name" hint="Shown under Runs on">
          <Input
            id="machine-name"
            autoFocus
            className="w-72 max-md:w-full"
            placeholder="vps"
            value={form.name}
            onChange={set('name')}
            disabled={running}
          />
        </Field>
        <Field
          id="machine-destination"
          label="Destination"
          hint={
            <>
              Or a <code className="font-mono">Host</code> from <code className="font-mono">~/.ssh/config</code>
            </>
          }
        >
          <Input
            id="machine-destination"
            className="w-72 max-md:w-full"
            placeholder="user@host"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={form.destination}
            onChange={set('destination')}
            disabled={running}
          />
        </Field>
        <Field id="machine-port" label="Port" hint="If not 22">
          <Input
            id="machine-port"
            className="w-20"
            placeholder="22"
            inputMode="numeric"
            value={form.port}
            onChange={set('port')}
            disabled={running}
          />
        </Field>
        <button type="submit" hidden />
      </form>
      {touched && problem && (
        <p role="alert" className="text-xs text-danger">
          {problem}
        </p>
      )}
      {test && <Steps test={test} />}
      <div className="flex flex-wrap gap-2">
        {editing ? (
          <Button variant="primary" onClick={save}>
            Save
          </Button>
        ) : running ? (
          <Button variant="primary" disabled>
            <Spinner />
            {action === 'install' ? 'Installing…' : 'Testing…'}
          </Button>
        ) : failed ? (
          <>
            {test.canInstall && (
              <Button variant="primary" icon="download" onClick={() => start('install')}>
                Install Silt on {form.name}
              </Button>
            )}
            <Button variant={test.canInstall ? 'secondary' : 'primary'} onClick={() => start(action)}>
              Try again
            </Button>
            <Button
              onClick={() => {
                addMachine(fieldsOf(form))
                onDone()
              }}
            >
              Add anyway
            </Button>
          </>
        ) : (
          <Button variant="primary" onClick={() => start()}>
            Test and add
          </Button>
        )}
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </section>
  )
}

/** Settings › Machines: where the desktop app's window can show Silt from, and computers to reach over SSH (ADR 0108). */
export function MachinesSettings() {
  const { backends, current } = useBackends()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const [form, setForm] = useState<{ editing?: Backend } | undefined>()
  const close = useCallback(() => setForm(undefined), [])
  const shown = backends.find((backend) => backend.id === current)
  // A machine's own page can't change the list: the window only takes that from This computer's.
  const remote = shown?.kind === 'ssh'
  const machines = backends.filter((backend) => backend.kind === 'ssh')
  const local = backends.filter((backend) => backend.kind !== 'ssh')
  const taken = machines.map((machine) => machine.label).filter((name) => name !== form?.editing?.label)
  // The build machines are offered: the `silt` the window runs here, which may differ from the daemon's.
  const latest = local.find((backend) => backend.kind === 'native')?.version
  const here = latest ?? (!remote && status.data?.version)
  const version = here && `silt ${here}`
  return (
    <section aria-label="Machines" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className={SETTINGS_HEADING}>Machines</h2>
        {!remote && !form && machines.length > 0 && (
          <span className="ml-auto">
            <Button icon="plus" onClick={() => setForm({})}>
              Add a machine
            </Button>
          </span>
        )}
      </div>
      <p className="text-sm text-fg-2">
        This window shows Silt from one machine at a time, and can list every connected machine’s threads together. Add a computer
        you reach over SSH, such as a VPS. Each machine keeps its own threads; hand one off through a server to move it.
      </p>
      {remote && (
        <Banner tone="info">
          This is <code className="font-mono">{shown.label}</code>’s Silt, shown over SSH. Add, change and remove machines from
          This computer.
        </Banner>
      )}
      <div className="rounded-md border border-line bg-elevated">
        {local.map((backend) => (
          <Row
            key={backend.id}
            name={backend.label}
            icon="laptop"
            status={<Status backend={backend} shown={backend.id === current} />}
            colour={<ColourPicker backend={backend} />}
            meta={
              backend.kind === 'wsl'
                ? 'A WSL distribution on this computer'
                : joined([version, remote ? 'this window runs here' : 'its daemon runs here'])
            }
            actions={
              backend.id !== current && (
                <Button size="sm" onClick={() => switchBackend(backend.id)}>
                  Open
                </Button>
              )
            }
          />
        ))}
        {machines.map((machine) => (
          <MachineRow
            key={machine.id}
            machine={machine}
            shown={machine.id === current}
            editable={!remote}
            latest={latest}
            onEdit={() => setForm({ editing: machine })}
          />
        ))}
      </div>
      {form ? (
        <MachineForm key={form.editing?.id ?? 'new'} editing={form.editing} taken={taken} onDone={close} />
      ) : (
        !remote &&
        machines.length === 0 && (
          <div className="rounded-md border border-dashed border-line px-4 py-8 text-center">
            <p className="text-sm font-medium text-fg-1">Add a computer you reach over SSH</p>
            <div className="mt-1 text-sm text-fg-2">
              A VPS or a machine at home, with Silt installed. Its threads, agents and files show in this window, without a
              server.
            </div>
            <div className="mt-4 flex justify-center">
              <Button variant="primary" size="lg" icon="plus" onClick={() => setForm({})}>
                Add a machine
              </Button>
            </div>
          </div>
        )
      )}
      {backends.length > 1 && <ThreadsFromMachines />}
      <p className="text-xs text-fg-2">
        Silt connects with this computer’s <code className="font-mono">ssh</code>, and your keys, agent and{' '}
        <code className="font-mono">~/.ssh/config</code>. It never asks for a password, so a machine needs a key that signs in
        without a prompt.
      </p>
    </section>
  )
}

const COLOUR_NAMES: Record<MachineColour, string> = {
  teal: 'Teal',
  plum: 'Plum',
  blue: 'Blue',
  rose: 'Rose',
  sand: 'Sand',
  ink: 'Ink',
}

/** A machine's colour, which its threads are marked with (ADR 0136), and the others to pick from. */
function ColourPicker({ backend }: { backend: Backend }) {
  const colour = backend.colour ?? 'ink'
  return (
    <Popover
      label={`${backend.label}’s colour`}
      triggerLabel={`${backend.label}’s colour: ${COLOUR_NAMES[colour]}`}
      triggerValue={colour}
      align="right"
      triggerClassName="inline-flex h-6 items-center gap-1.5 rounded-md border border-line bg-elevated px-2 text-xs text-fg-1 hover:bg-surface-2 aria-expanded:border-accent"
      panelClassName="p-2"
      trigger={
        <>
          <span aria-hidden="true" className="h-[9px] w-[9px] rounded-[2px]" style={{ background: colourOf(colour) }} />
          {COLOUR_NAMES[colour]}
        </>
      }
    >
      {(close) => (
        <div role="radiogroup" aria-label={`${backend.label}’s colour`} className="flex gap-1.5">
          {MACHINE_COLOURS.map((each) => (
            <button
              key={each}
              type="button"
              role="radio"
              aria-checked={each === colour}
              aria-label={COLOUR_NAMES[each]}
              title={COLOUR_NAMES[each]}
              onClick={() => {
                close()
                if (each !== colour) setMachineColour(backend.id, each)
              }}
              className={cx(
                'h-5 w-5 rounded-sm max-md:h-9 max-md:w-9',
                each === colour && 'shadow-[0_0_0_2px_var(--bg-elevated),0_0_0_3.5px_var(--fg-1)]',
              )}
              style={{ background: colourOf(each) }}
            />
          ))}
        </div>
      )}
    </Popover>
  )
}

const MARK_OPTIONS: { value: Mark; label: string; detail: string }[] = [
  { value: 'chip', label: 'Machine chip', detail: 'its name in a small chip' },
  { value: 'colour-chip', label: 'Chip in its colour', detail: 'the chip filled with the machine’s colour' },
  { value: 'line', label: 'In the line', detail: 'after the thread’s state, in words' },
  { value: 'stripe', label: 'Colour', detail: 'a stripe at the row’s edge' },
  { value: 'initial', label: 'Initial', detail: 'its first letter on its colour' },
  { value: 'nothing', label: 'Nothing', detail: 'when the threads never mix up; the tooltip still says' },
]

/** How a thread's mark looks, beside its option. */
function MarkPreview({ mark, backend }: { mark: Mark; backend: Backend }) {
  const marked = { name: machineName(backend), colour: colourOf(backend.colour), mark }
  return (
    <span aria-hidden="true" className="ml-auto inline-flex items-center gap-1.5 text-2xs whitespace-nowrap text-fg-2">
      {mark === 'stripe' && <span className="h-3.5 w-[3px] rounded-sm" style={{ background: marked.colour }} />}
      {(mark === 'chip' || mark === 'colour-chip') && <MachineChip marked={marked} />}
      Working{mark === 'line' && ` · on ${marked.name}`}
      {mark === 'initial' && <MachineInitial marked={marked} />}
    </span>
  )
}

/** Whether the sidebar lists every connected machine's threads, and how each thread's machine is marked (ADR 0136). */
function ThreadsFromMachines() {
  const view = useMachinesView()
  const { backends, current } = useBackends()
  const example = backends.find((backend) => backend.id !== current) ?? backends[0]
  return (
    <section aria-label="Threads from other machines" className="mt-2 flex flex-col gap-3">
      <h3 className="text-sm font-medium">Threads from other machines</h3>
      <div className="flex flex-col rounded-md border border-line bg-elevated">
        <div className="flex flex-wrap items-center gap-3 border-b border-line-subtle px-4 py-3">
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="text-sm">List</span>
            <span className="text-2xs text-fg-2">The machine menu’s choice, the same on every machine’s page.</span>
          </span>
          <span role="group" aria-label="List" className="inline-flex gap-1.5">
            <Chip selected={view.list === 'all'} onClick={() => setMachinesList('all')}>
              All machines
            </Chip>
            <Chip selected={view.list === 'one'} onClick={() => setMachinesList('one')}>
              One machine at a time
            </Chip>
          </span>
        </div>
        <fieldset className="flex flex-col gap-1 border-b border-line-subtle px-4 py-3">
          <legend className="sr-only">Mark each thread’s machine</legend>
          <span className="text-sm">Mark each thread’s machine</span>
          <span className="mb-1 text-2xs text-fg-2">How a thread on another machine stands apart in the list and Needs you.</span>
          {MARK_OPTIONS.map((option) => (
            <label key={option.value} className="flex cursor-pointer items-center gap-2 py-1 text-sm max-md:min-h-11">
              <input
                type="radio"
                name="mark"
                className="accent-[var(--accent)]"
                checked={view.mark === option.value}
                onChange={() => setMark(option.value, view.markHere)}
              />
              <span className="min-w-0">
                {option.label} <span className="text-2xs text-fg-2">{option.detail}</span>
              </span>
              {example && <MarkPreview mark={option.value} backend={example} />}
            </label>
          ))}
        </fieldset>
        <label className="flex cursor-pointer items-start gap-2 px-4 py-3 text-sm max-md:min-h-11">
          <input
            type="checkbox"
            className="mt-1 accent-[var(--accent)]"
            checked={view.markHere}
            disabled={view.mark === 'nothing'}
            onChange={(event) => setMark(view.mark, event.target.checked)}
          />
          <span className="flex flex-col">
            <span>Mark the shown machine’s threads too</span>
            <span className="text-2xs text-fg-2">Off: only threads on another machine are marked.</span>
          </span>
        </label>
      </div>
    </section>
  )
}
