// Settings › Text and layout (ADR 0107): how big and how dense the UI is,
// over any theme, with a preview drawn by the components the app uses.

import { type ReactNode, useState } from 'react'
import type { DiffLayout, TextSettings } from '../api'
import { inDesktop } from '../desktop'
import { message } from '../query'
import { hosted } from '../served'
import { SETTINGS_HEADING } from '../settingsContext'
import { formatKeys } from '../shortcuts'
import { RANGES, type SizeKey, type TextKey, changed, stepped, useTextSettings } from '../text'
import { Banner, Button, Input, Select, Skeleton, cx } from '../ui'
import { DiffView } from './DiffView'
import { Markdown } from './Markdown'

const GROUP = 'flex justify-between pt-2 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase'

const REPLY = `Yes. Long lines wrap at the panel's edge now, and each line's number stays on its first row. The row is \`whitespace-pre-wrap\` with \`overflow-wrap: anywhere\`:

\`\`\`tsx
const row = cx('relative min-w-0', wrap && 'whitespace-pre-wrap') // => != ===
\`\`\``

const DIFF = `diff --git a/web/src/components/DiffView.tsx b/web/src/components/DiffView.tsx
--- a/web/src/components/DiffView.tsx
+++ b/web/src/components/DiffView.tsx
@@ -468,3 +468,3 @@
   return (
-    <span className="relative min-w-0 pr-3 whitespace-pre">
+    <span className="relative min-w-0 pr-3 whitespace-pre-wrap [overflow-wrap:anywhere]">
`

/** A reply, a code block and a diff at the sizes set, as a thread draws them. */
function Preview({ shown }: { shown: ReturnType<typeof useTextSettings>['shown'] }) {
  return (
    <section aria-label="preview" className="overflow-hidden rounded-md border border-line bg-elevated">
      <p className="flex justify-between gap-3 border-b border-line-subtle px-4 py-1.5 font-mono text-2xs text-fg-3">
        <span>Preview</span>
        <span>
          conversation {shown.chat_size}px · code {shown.code_size}px
        </span>
      </p>
      <div className="flex flex-col gap-[calc(14px*var(--density))] px-4 py-3">
        <p className="ml-auto w-fit max-w-[85%] rounded-lg bg-surface-2 px-3.5 py-2.5 text-chat leading-[1.45] whitespace-pre-wrap">
          Can the diff wrap long lines instead of cutting them off?
        </p>
        <Markdown text={REPLY} />
        <DiffView text={DIFF} layout="unified" />
      </div>
    </section>
  )
}

/** One setting: what it is, a mark and a Reset when it's changed, and its control. */
function Row({
  label,
  says,
  modified,
  onReset,
  children,
}: {
  label: string
  says?: ReactNode
  modified: boolean
  onReset: () => void
  children: ReactNode
}) {
  return (
    <div
      className={cx(
        'flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line-subtle px-3.5 py-2.5 last:border-b-0',
        modified && 'shadow-[inset_2px_0_0_var(--accent)]',
      )}
      data-modified={modified || undefined}
    >
      <div className="flex min-w-[min(100%,240px)] flex-1 flex-col gap-0.5">
        <span className="text-sm font-medium text-fg-1">{label}</span>
        {says && <span className="text-xs text-fg-2">{says}</span>}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {modified && (
          <button
            type="button"
            onClick={onReset}
            aria-label={`reset ${label.toLowerCase()}`}
            className="text-2xs text-accent-fg underline underline-offset-2 hover:text-fg-1 max-md:min-h-11"
          >
            Reset
          </button>
        )}
        {children}
      </div>
    </div>
  )
}

/** A size stepped by its range, as − 13 px +. */
function Stepper({
  label,
  value,
  unit,
  range,
  onChange,
}: {
  label: string
  value: number
  unit: string
  range: (typeof RANGES)[SizeKey]
  onChange: (next: number) => void
}) {
  const button =
    'inline-flex h-full w-7 items-center justify-center text-fg-2 hover:bg-surface-2 hover:text-fg-1 disabled:text-fg-3 disabled:hover:bg-transparent max-md:w-11'
  return (
    <span
      role="group"
      aria-label={label}
      className="inline-flex h-7 items-stretch overflow-hidden rounded-sm border border-line bg-elevated max-md:h-11"
    >
      <button
        type="button"
        aria-label={`smaller ${label}`}
        disabled={value <= range.min}
        onClick={() => onChange(stepped(range, value, -1))}
        className={button}
      >
        −
      </button>
      <output
        aria-live="polite"
        className="flex min-w-[64px] items-center justify-center border-x border-line-subtle font-mono text-xs text-fg-1"
      >
        {value}
        {unit}
      </output>
      <button
        type="button"
        aria-label={`bigger ${label}`}
        disabled={value >= range.max}
        onClick={() => onChange(stepped(range, value, 1))}
        className={button}
      >
        +
      </button>
    </span>
  )
}

/** One of a few choices, as the theme mode's buttons are. */
function Choices<T extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: { id: T; label: string }[]
  onChange: (next: T) => void
}) {
  return (
    <span role="radiogroup" aria-label={label} className="inline-flex overflow-hidden rounded-sm border border-line text-xs">
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          role="radio"
          aria-checked={value === option.id}
          onClick={() => onChange(option.id)}
          className={cx(
            'px-2.5 py-1 not-first:border-l not-first:border-line-subtle max-md:min-h-11',
            value === option.id ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2',
          )}
        >
          {option.label}
        </button>
      ))}
    </span>
  )
}

const FONT_NAME = /^[A-Za-z0-9 _-]{1,64}$/

/** The theme's font, the system's, or one by name. */
function FontPicker({
  label,
  value,
  onChange,
}: {
  label: string
  value: string | null
  onChange: (next: string | null) => void
}) {
  const kind = value === null ? 'theme' : value === 'system' ? 'system' : 'named'
  const [named, setNamed] = useState(kind === 'named' ? (value ?? '') : '')
  const [picking, setPicking] = useState(false)
  const showInput = kind === 'named' || picking
  const valid = FONT_NAME.test(named.trim())
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <Select
        aria-label={label}
        value={showInput ? 'named' : kind}
        onChange={(event) => {
          const picked = event.target.value
          setPicking(picked === 'named')
          if (picked === 'theme') onChange(null)
          else if (picked === 'system') onChange('system')
        }}
        className="w-[160px] text-xs"
      >
        <option value="theme">The theme's</option>
        <option value="system">The system's</option>
        <option value="named">One by name…</option>
      </Select>
      {showInput && (
        <form
          className="flex gap-1.5"
          onSubmit={(event) => {
            event.preventDefault()
            if (!valid) return
            setPicking(false)
            onChange(named.trim())
          }}
        >
          <Input
            aria-label={`${label} name`}
            aria-invalid={named.length > 0 && !valid}
            placeholder="JetBrains Mono"
            value={named}
            onChange={(event) => setNamed(event.target.value)}
            className="w-[160px] text-xs"
          />
          <Button type="submit" disabled={!valid || named.trim() === value}>
            Use
          </Button>
        </form>
      )}
    </span>
  )
}

export function TextLayoutSettings() {
  const { text, shown, ready, error: unread, set, change } = useTextSettings()
  const [error, setError] = useState<string>()
  const save = (next: Promise<void>) => {
    setError(undefined)
    next.catch((failure: unknown) => setError(message(failure)))
  }
  const put = <K extends TextKey>(key: K, value: TextSettings[K]) => save(set(key, value))
  const reset = (key: TextKey) => save(change({ ...text, [key]: null }))
  const row = (key: TextKey, label: string, says: ReactNode, control: ReactNode) => (
    <Row key={key} label={label} says={says} modified={changed(text, key)} onReset={() => reset(key)}>
      {control}
    </Row>
  )
  const size = (key: SizeKey, label: string, says: ReactNode) =>
    row(
      key,
      label,
      says,
      <Stepper
        label={label.toLowerCase()}
        value={shown[key]}
        unit={key === 'zoom' ? '%' : ' px'}
        range={RANGES[key]}
        onChange={(next) => put(key, next)}
      />,
    )
  const anyChanged = Object.keys(text).some((key) => changed(text, key as TextKey))
  const desktop = inDesktop()
  return (
    <section aria-label="Text and layout" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2.5">
        <h2 className={SETTINGS_HEADING}>Text and layout</h2>
        <span className="font-mono text-2xs text-fg-2">
          {hosted() ? 'kept in this browser' : 'this machine · appearance.json'}
        </span>
        {anyChanged && (
          <Button className="ml-auto" onClick={() => save(change({}))}>
            Reset all
          </Button>
        )}
      </div>
      <p className="text-sm text-fg-2">
        How big and how dense Silt is. These apply over any theme; the theme still sets colours.
      </p>
      {error && (
        <Banner tone="danger" urgent>
          Couldn't save: {error}. Silt shows what was saved before.
        </Banner>
      )}
      {unread && <Banner tone="danger">Couldn't read this machine's settings: {message(unread)}</Banner>}
      {!desktop && (
        <Banner>
          To zoom everything, use your browser's own zoom ({formatKeys('mod+=')}). The desktop app keeps its zoom here.
          {hosted() && ' These settings are kept in this browser only.'}
        </Banner>
      )}
      {!ready ? (
        <div aria-busy="true" className="flex flex-col gap-2">
          <Skeleton className="h-40 w-full" />
          {[0, 1, 2, 3].map((index) => (
            <Skeleton key={index} className="h-11 w-full" />
          ))}
        </div>
      ) : (
        <>
          <Preview shown={shown} />
          <p className={GROUP}>Size</p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {desktop &&
              size(
                'zoom',
                'Zoom',
                <>
                  Everything in the window, as one. {formatKeys('mod+=')} and {formatKeys('mod+-')} change it,{' '}
                  {formatKeys('mod+0')} resets it.
                </>,
              )}
            {size('ui_size', 'Interface text', 'Sidebar, headers, buttons and lists. Small print and headings follow it.')}
            {size('chat_size', 'Conversation text', "The agent's replies, your messages and docs.")}
            {size('code_size', 'Code text', 'Code blocks, diffs and tool output.')}
          </div>
          <p className={GROUP}>
            <span>Fonts</span>
            <span>over the theme's</span>
          </p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {row(
              'ui_font',
              'Interface font',
              'An installed font, by its name.',
              <FontPicker label="interface font" value={shown.ui_font} onChange={(next) => put('ui_font', next)} />,
            )}
            {row(
              'code_font',
              'Code font',
              undefined,
              <FontPicker label="code font" value={shown.code_font} onChange={(next) => put('code_font', next)} />,
            )}
            {row(
              'ligatures',
              'Ligatures in code',
              'Draws => and != as one glyph where the font has them.',
              <input
                type="checkbox"
                aria-label="ligatures in code"
                checked={shown.ligatures}
                onChange={(event) => put('ligatures', event.target.checked)}
                className="h-4 w-4 accent-(--accent) max-md:h-6 max-md:w-6"
              />,
            )}
          </div>
          <p className={GROUP}>Spacing</p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {row(
              'density',
              'Density',
              "Rows in the sidebar, and the space between a thread's turns.",
              <Choices
                label="density"
                value={shown.density}
                onChange={(next) => put('density', next)}
                options={[
                  { id: 'compact', label: 'Compact' },
                  { id: 'regular', label: 'Default' },
                  { id: 'roomy', label: 'Roomy' },
                ]}
              />,
            )}
            {row(
              'line_height',
              'Conversation line height',
              undefined,
              <Choices
                label="conversation line height"
                value={shown.line_height}
                onChange={(next) => put('line_height', next)}
                options={[
                  { id: 'tight', label: 'Tight' },
                  { id: 'regular', label: 'Default' },
                  { id: 'loose', label: 'Loose' },
                ]}
              />,
            )}
            {row(
              'width',
              'Conversation width',
              "How wide a thread's column grows.",
              <Choices
                label="conversation width"
                value={shown.width}
                onChange={(next) => put('width', next)}
                options={[
                  { id: 'narrow', label: '640' },
                  { id: 'regular', label: '720' },
                  { id: 'wide', label: '880' },
                  { id: 'full', label: 'Full' },
                ]}
              />,
            )}
          </div>
          <p className={GROUP}>Code and diffs</p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {row(
              'diff_layout',
              'Diff layout',
              'What a diff opens in. Auto puts them side by side where there is room; each diff can still switch.',
              <Choices
                label="diff layout"
                value={shown.diff_layout ?? 'auto'}
                onChange={(next) => put('diff_layout', next === 'auto' ? null : (next as DiffLayout))}
                options={[
                  { id: 'auto', label: 'Auto' },
                  { id: 'unified', label: 'Unified' },
                  { id: 'split', label: 'Split' },
                ]}
              />,
            )}
            {row(
              'wrap',
              'Wrap long lines',
              'In a unified diff. Off, long lines scroll sideways.',
              <input
                type="checkbox"
                aria-label="wrap long lines"
                checked={shown.wrap}
                onChange={(event) => put('wrap', event.target.checked)}
                className="h-4 w-4 accent-(--accent) max-md:h-6 max-md:w-6"
              />,
            )}
            {row(
              'tab_width',
              'Tab width',
              'Columns a tab takes in code and diffs.',
              <Choices
                label="tab width"
                value={shown.tab_width}
                onChange={(next) => put('tab_width', next)}
                options={[
                  { id: 2, label: '2' },
                  { id: 4, label: '4' },
                  { id: 8, label: '8' },
                ]}
              />,
            )}
          </div>
          <p className={GROUP}>Conversation</p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {row(
              'detail',
              'Detail',
              "How much of the agent's work a thread shows. A thread's own menu changes it too.",
              <Choices
                label="detail"
                value={shown.detail}
                onChange={(next) => put('detail', next)}
                options={[
                  { id: 'normal', label: 'Normal' },
                  { id: 'verbose', label: 'Verbose' },
                ]}
              />,
            )}
            {row(
              'grouping',
              'Grouping',
              "A finished turn's work folded together, or each run between the agent's words.",
              <Choices
                label="grouping"
                value={shown.grouping}
                onChange={(next) => put('grouping', next)}
                options={[
                  { id: 'turn', label: 'By turn' },
                  { id: 'run', label: 'By run' },
                ]}
              />,
            )}
            {row(
              'motion',
              'Reduce motion',
              'Cuts animations short.',
              <Choices
                label="reduce motion"
                value={shown.motion}
                onChange={(next) => put('motion', next)}
                options={[
                  { id: 'system', label: 'As the system asks' },
                  { id: 'reduced', label: 'Always' },
                  { id: 'full', label: 'Never' },
                ]}
              />,
            )}
          </div>
          {!hosted() && (
            <div className="rounded-md border border-dashed border-line px-3.5 py-3 text-sm text-fg-2">
              Kept in <code className="font-mono text-xs">appearance.json</code> in Silt's data directory, beside the theme you
              picked. Your other machines keep their own.
            </div>
          )}
        </>
      )}
    </section>
  )
}
