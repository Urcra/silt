// Design systems in Settings (ADR 0088): every system here, one system's page with its components drawn live, and the
// threads whose agents make or change one.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { type RefObject, useEffect, useMemo, useRef, useState } from 'react'
import {
  api,
  type DesignComponentView,
  type DesignDetail,
  type DesignFrom,
  type DesignKeep,
  type DesignSummary,
  type PromptAttachment,
} from '../api'
import { readAttachment } from '../attachments'
import type { DesignSearch } from '../designSearch'
import { keys, message } from '../query'
import { SETTINGS_HEADING } from '../settingsContext'
import { useShownTheme } from '../theme'
import { partDocument } from '../typed'
import { Button, Dialog, EmptyState, Icon, type IconName, Input, Select, Tabs, Tag, Textarea, cx } from '../ui'
import { Markdown } from './Markdown'
import { Page } from './Page'

/** The search that opens `summary`'s page. */
function pageSearch(summary: DesignSummary, clone?: string): DesignSearch {
  return {
    source: summary.source,
    name: summary.name,
    ...(clone && summary.source === 'project' ? { clone } : {}),
    ...(summary.unmerged && summary.thread ? { thread: summary.thread } : {}),
  }
}

function Swatches({ colors, size = 18 }: { colors: string[]; size?: number }) {
  return (
    <span aria-hidden="true" className="grid shrink-0 grid-cols-4 gap-[3px]">
      {colors.slice(0, 8).map((color, index) => (
        <span key={index} className="rounded-sm border border-line-subtle" style={{ width: size, height: size, background: color }} />
      ))}
    </span>
  )
}

function SystemRow({ summary, clone }: { summary: DesignSummary; clone?: string }) {
  return (
    <Link
      to="/settings/design-systems/system"
      search={pageSearch(summary, clone)}
      className="flex items-center gap-3.5 rounded-md border border-line bg-elevated px-3.5 py-3 hover:border-line-strong"
    >
      <Swatches colors={summary.swatches} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="text-[14px] font-medium text-fg-1">{summary.title}</span>
          <span className="font-mono text-2xs text-fg-3">{summary.name}</span>
          {summary.uses.map((use) => (
            <Tag key={use} tone="accent">
              {use}
            </Tag>
          ))}
          {summary.thread && <Tag>{summary.unmerged ? 'being made' : 'being changed'}</Tag>}
        </span>
        <span className="truncate font-mono text-2xs text-fg-3">{summary.dir}</span>
      </span>
      <span className="shrink-0 text-right font-mono text-2xs text-fg-3 max-md:hidden">
        {summary.colors} colours · {summary.type_styles} type styles
        <br />
        {summary.components} components
      </span>
      <Icon name="chevronRight" className="shrink-0 text-fg-2" />
    </Link>
  )
}

/** Settings › Design systems: the repository's systems, project by project, then the person's own. */
export function DesignSystemsSettings() {
  const catalog = useQuery({ queryKey: keys.designCatalog, queryFn: api.designCatalog })
  const [making, setMaking] = useState(false)
  const groups = catalog.data?.groups ?? []
  const count = groups.reduce((sum, group) => sum + group.systems.length, 0)
  return (
    <section aria-label="Design systems" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className={SETTINGS_HEADING}>
          Design systems {catalog.data && <span className="ml-1 font-mono text-2xs font-normal text-fg-3">{count === 1 ? '1 system' : `${count} systems`}</span>}
        </h2>
        <Button icon="plus" onClick={() => setMaking(true)}>
          New design system
        </Button>
      </div>
      <p className="text-sm text-fg-2">What an agent reads before it styles an artifact: rules, tokens and components. A project or workspace picks one; the rest are there to ask for by name.</p>
      {catalog.isError && <p className="text-sm text-danger">{message(catalog.error)}</p>}
      {groups.map((group) =>
        group.systems.length === 0 && !group.project ? null : (
          <div key={group.project?.repo ?? 'yours'} className="flex flex-col gap-2">
            <p className="pt-2 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">
              {group.project ? `${group.project.name} · in the repository` : 'Yours · synced to your devices'}
            </p>
            {group.systems.map((summary) => (
              <SystemRow key={`${summary.name}-${summary.thread ?? ''}`} summary={summary} clone={group.project?.clone} />
            ))}
          </div>
        ),
      )}
      {catalog.data && (
        <div className="rounded-md border border-dashed border-line px-3.5 py-3 text-sm text-fg-2">
          Agents make a design system from a project's code, a website, a brand guide or a description.{' '}
          <button type="button" className="text-accent-fg hover:text-fg-1" onClick={() => setMaking(true)}>
            New design system
          </button>
        </div>
      )}
      <NewDesignSystem open={making} onClose={() => setMaking(false)} />
    </section>
  )
}

/** The agents here, and the one picked: the first unless one is chosen. */
function useAgentChoice() {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const agents = choices.data?.agents ?? []
  const [agent, setAgent] = useState('')
  return { agents, clones: choices.data?.clones ?? [], agent: agent || agents[0]?.id || '', setAgent }
}

const SOURCES: { id: DesignFrom; icon: IconName; title: string; about: string }[] = [
  { id: 'code', icon: 'code', title: "A project's code", about: 'Its stylesheets, theme files, components, fonts and logos. The most faithful.' },
  { id: 'website', icon: 'link', title: 'A website, brand guide or screenshots', about: 'A link, a PDF or pictures you attach.' },
  { id: 'description', icon: 'list', title: 'A description', about: 'Start from words: a mood, a few colours, a typeface.' },
]

/** Asks an agent to make a design system, in a new thread it then opens. */
export function NewDesignSystem({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate()
  const queries = useQueryClient()
  const { agents, clones, agent, setAgent } = useAgentChoice()
  const [title, setTitle] = useState('')
  const [from, setFrom] = useState<DesignFrom>('code')
  const [clone, setClone] = useState('')
  const [link, setLink] = useState('')
  const [notes, setNotes] = useState('')
  const [keep, setKeep] = useState<DesignKeep>('repo')
  const [files, setFiles] = useState<PromptAttachment[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const project = clone || clones[0]?.path || ''
  const needsProject = from === 'code' || keep === 'repo'
  const ready =
    title.trim() !== '' &&
    agent !== '' &&
    (!needsProject || project !== '') &&
    (from !== 'website' || link.trim() !== '' || files.length > 0) &&
    (from !== 'description' || notes.trim() !== '' || files.length > 0)
  const start = async () => {
    setBusy(true)
    setError(undefined)
    try {
      const thread = await api.designStart({
        title: title.trim(),
        agent,
        keep,
        clone: needsProject ? project : null,
        from,
        link: from === 'website' && link.trim() ? link.trim() : null,
        notes: notes.trim() || null,
        attachments: files,
        change: null,
        name: null,
      })
      void queries.invalidateQueries({ queryKey: keys.designCatalog })
      onClose()
      void navigate({ to: '/t/$id', params: { id: thread } })
    } catch (failure) {
      setError(message(failure))
    } finally {
      setBusy(false)
    }
  }
  const attach = async (list: FileList | null) => {
    const read = await Promise.all([...(list ?? [])].map((file) => readAttachment(file)))
    setFiles((was) => [...was, ...read])
  }
  const dir = keep === 'repo' ? `.silt/design-systems/${slug(title) || 'name'}` : `your design systems · ${slug(title) || 'name'}`
  return (
    <Dialog open={open} onClose={onClose} title="New design system">
      <div className="flex flex-col gap-3.5 text-sm">
        <p className="text-fg-2">An agent builds it in a new thread. You watch it take shape beside the conversation, and change it by talking to the agent.</p>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-fg-2">Name</span>
          <Input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="Harbor" />
        </label>
        <fieldset className="flex flex-col gap-1.5">
          <legend className="mb-1 text-xs text-fg-2">Build it from</legend>
          {SOURCES.map((source) => (
            <label
              key={source.id}
              className={cx(
                'flex cursor-pointer items-start gap-2.5 rounded-md border px-3 py-2.5',
                from === source.id ? 'border-accent bg-accent-tint shadow-[inset_2px_0_0_var(--accent)]' : 'border-line bg-bg hover:border-line-strong',
              )}
            >
              <input type="radio" name="design-from" className="mt-1 accent-[var(--accent)]" checked={from === source.id} onChange={() => setFrom(source.id)} />
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="flex items-center gap-1.5 font-medium text-fg-1">
                  <Icon name={source.icon} className="text-fg-2" />
                  {source.title}
                </span>
                <span className="text-xs text-fg-2">{source.about}</span>
                {from === source.id && source.id === 'website' && (
                  <span className="mt-1.5 flex flex-col gap-1.5">
                    <Input aria-label="website" value={link} onChange={(event) => setLink(event.target.value)} placeholder="https://acme.example" />
                    <AttachFiles files={files} onAttach={(list) => void attach(list)} onRemove={(name) => setFiles((was) => was.filter((file) => file.name !== name))} />
                  </span>
                )}
                {from === source.id && source.id === 'description' && (
                  <Textarea aria-label="description" rows={3} className="mt-1.5" value={notes} onChange={(event) => setNotes(event.target.value)} placeholder="Calm and nautical: deep navy, one teal accent, a humanist sans." />
                )}
              </span>
            </label>
          ))}
        </fieldset>
        {needsProject && (
          <label className="flex flex-col gap-1">
            <span className="text-xs text-fg-2">Project</span>
            <Select aria-label="project" value={project} onChange={(event) => setClone(event.target.value)}>
              {clones.length === 0 && <option value="">No project on this machine</option>}
              {clones.map((candidate) => (
                <option key={candidate.path} value={candidate.path}>
                  {candidate.path.split('/').pop() || candidate.repo}
                </option>
              ))}
            </Select>
          </label>
        )}
        {from !== 'description' && (
          <label className="flex flex-col gap-1">
            <span className="text-xs text-fg-2">
              Anything else <span className="font-mono text-2xs text-fg-3">optional</span>
            </span>
            <Textarea rows={2} value={notes} onChange={(event) => setNotes(event.target.value)} placeholder="Only the marketing site's look, not the admin app" />
          </label>
        )}
        <div className="flex flex-col gap-1">
          <span className="text-xs text-fg-2">Keep it</span>
          <div role="radiogroup" aria-label="keep it" className="inline-flex self-start overflow-hidden rounded-sm border border-line">
            {(['repo', 'personal'] as const).map((choice) => (
              <button
                key={choice}
                type="button"
                role="radio"
                aria-checked={keep === choice}
                onClick={() => setKeep(choice)}
                className={cx('px-3 py-1 text-xs max-md:min-h-11', keep === choice ? 'bg-surface-2 font-medium text-fg-1' : 'text-fg-2 hover:bg-surface-1')}
              >
                {choice === 'repo' ? 'In the repository' : 'Yours, on every device'}
              </button>
            ))}
          </div>
          <span className="font-mono text-2xs text-fg-3">{keep === 'repo' ? `${dir} · part of the thread's changes` : `${dir} · synced`}</span>
        </div>
        {error && <p className="text-xs text-danger">{error}</p>}
        <div className="-mx-5 -mb-4 flex flex-wrap items-center justify-between gap-2 border-t border-line bg-bg px-5 py-3">
          <AgentSelect agents={agents} agent={agent} onChange={setAgent} />
          <span className="flex items-center gap-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" disabled={!ready || busy} onClick={() => void start()}>
              {busy ? 'Starting…' : 'Start building'}
            </Button>
          </span>
        </div>
      </div>
    </Dialog>
  )
}

function AttachFiles({ files, onAttach, onRemove }: { files: PromptAttachment[]; onAttach: (files: FileList | null) => void; onRemove: (name: string) => void }) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <Button size="sm" icon="attach" onClick={() => input.current?.click()}>
        Attach a guide or screenshots
      </Button>
      <input ref={input} type="file" multiple accept="image/*,application/pdf" className="hidden" onChange={(event) => onAttach(event.target.files)} />
      {files.map((file) => (
        <Tag key={file.name}>
          {file.name}
          <button type="button" aria-label={`remove ${file.name}`} className="ml-1" onClick={() => onRemove(file.name)}>
            ×
          </button>
        </Tag>
      ))}
    </span>
  )
}

function AgentSelect({ agents, agent, onChange }: { agents: { id: string; name: string }[]; agent: string; onChange: (agent: string) => void }) {
  return (
    <label className="flex items-center gap-1.5 text-xs text-fg-2">
      <Icon name="agent" />
      <Select aria-label="agent" value={agent} onChange={(event) => onChange(event.target.value)} className="text-xs">
        {agents.map((candidate) => (
          <option key={candidate.id} value={candidate.id}>
            {candidate.name}
          </option>
        ))}
      </Select>
    </label>
  )
}

/** A system's folder name as the daemon will make it from what it's called. */
function slug(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
}

/** Asks an agent to change a system, each change in a thread of its own. */
function ChangeDesignSystem({ open, onClose, detail, search }: { open: boolean; onClose: () => void; detail: DesignDetail; search: DesignSearch }) {
  const navigate = useNavigate()
  const { agents, agent, setAgent } = useAgentChoice()
  const [change, setChange] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const start = async () => {
    setBusy(true)
    setError(undefined)
    try {
      const thread = await api.designStart({
        title: detail.summary.title,
        name: detail.summary.name,
        agent,
        keep: detail.summary.source === 'personal' ? 'personal' : 'repo',
        clone: search.clone ?? null,
        from: null,
        link: null,
        notes: null,
        attachments: [],
        change: change.trim(),
      })
      onClose()
      void navigate({ to: '/t/$id', params: { id: thread } })
    } catch (failure) {
      setError(message(failure))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open={open} onClose={onClose} title={`Change ${detail.summary.title}`}>
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-fg-2">A new thread for this change, so it has its own review. The agent reads the system first and changes only what you ask.</p>
        <Textarea aria-label="what to change" rows={4} value={change} onChange={(event) => setChange(event.target.value)} placeholder="Add a dark theme, and a table component from src/ui/Table.tsx" />
        {error && <p className="text-xs text-danger">{error}</p>}
        <div className="-mx-5 -mb-4 flex flex-wrap items-center justify-between gap-2 border-t border-line bg-bg px-5 py-3">
          <AgentSelect agents={agents} agent={agent} onChange={setAgent} />
          <span className="flex items-center gap-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" disabled={!change.trim() || !agent || busy} onClick={() => void start()}>
              {busy ? 'Starting…' : 'Start the change'}
            </Button>
          </span>
        </div>
      </div>
    </Dialog>
  )
}

/** How wide an element is now. */
function useWidth<T extends HTMLElement>(): [RefObject<T | null>, number] {
  const ref = useRef<T>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver(() => setWidth(Math.round(element.getBoundingClientRect().width)))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  return [ref, width]
}

/**
 * Markup drawn in a sealed frame with a system's compiled CSS, as an artifact gets it, at `width`, as tall as it
 * says its content is.
 */
export function SystemFrame({ html, css, theme, width, title, scale = 1, least = 40 }: { html: string; css: string; theme: 'light' | 'dark'; width: number; title: string; scale?: number; least?: number }) {
  const frame = useRef<HTMLIFrameElement>(null)
  const [height, setHeight] = useState(least)
  useEffect(() => {
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; height?: unknown } | null
      if (said?.silt !== 'grown' || typeof said.height !== 'number' || event.source !== frame.current?.contentWindow) return
      setHeight(Math.min(Math.max(Math.round(said.height), least), 4000))
    }
    window.addEventListener('message', heard)
    return () => window.removeEventListener('message', heard)
  }, [least])
  const document = useMemo(() => (width > 0 ? partDocument(html, undefined, width, least, css, theme, undefined, true) : ''), [html, css, theme, width, least])
  if (width <= 0) return null
  return (
    <div style={{ width: width * scale, height: height * scale, overflow: 'hidden' }}>
      <iframe
        ref={frame}
        title={title}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        srcDoc={document}
        style={{ width, height, border: 0, transform: scale === 1 ? undefined : `scale(${scale})`, transformOrigin: '0 0', display: 'block' }}
      />
    </div>
  )
}

const PREVIEW_PAD = '<div style="padding:14px 16px;display:flex;flex-wrap:wrap;gap:8px;align-items:center">'

function ComponentCard({ component, css, theme, phone }: { component: DesignComponentView; css: string; theme: 'light' | 'dark'; phone: boolean }) {
  const [box, width] = useWidth<HTMLDivElement>()
  const [shown, setShown] = useState<'preview' | 'markup' | 'guide'>('preview')
  return (
    <article id={`component-${component.name}`} className="scroll-mt-16 overflow-hidden rounded-md border border-line bg-elevated">
      <header className="flex items-start gap-2 border-b border-line-subtle px-3 py-2">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="font-mono text-xs text-fg-1">{component.name}</span>
          {component.about && <span className="text-xs text-fg-2">{component.about}</span>}
        </span>
        <Tabs
          label={`${component.name} view`}
          tabs={[
            { id: 'preview' as const, label: 'Preview' },
            ...(component.guide ? [{ id: 'guide' as const, label: 'Guidelines' }] : []),
            { id: 'markup' as const, label: 'Markup' },
          ]}
          value={shown}
          onChange={setShown}
        />
      </header>
      <div ref={box} className={cx(phone && 'flex justify-center bg-surface-1')}>
        {shown === 'preview' && (
          <SystemFrame html={`${PREVIEW_PAD}${component.html}</div>`} css={css} theme={theme} width={phone ? Math.min(390, width) : width} title={`${component.name} preview`} />
        )}
        {shown === 'markup' && <pre className="max-h-96 overflow-auto px-3 py-2.5 font-mono text-2xs leading-relaxed whitespace-pre-wrap text-fg-1">{component.html}</pre>}
        {shown === 'guide' && component.guide && <Markdown text={component.guide} className="px-4 py-3 text-sm" />}
      </div>
    </article>
  )
}

function Segmented<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { id: T; label: string }[]; onChange: (value: T) => void }) {
  return (
    <span role="radiogroup" aria-label={label} className="inline-flex overflow-hidden rounded-sm border border-line">
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          role="radio"
          aria-checked={value === option.id}
          onClick={() => onChange(option.id)}
          className={cx('px-2.5 py-0.5 text-xs max-md:min-h-11', value === option.id ? 'bg-surface-2 font-medium text-fg-1' : 'text-fg-2 hover:bg-surface-1')}
        >
          {option.label}
        </button>
      ))}
    </span>
  )
}

function Components({ detail, theme, phone }: { detail: DesignDetail; theme: 'light' | 'dark'; phone: boolean }) {
  if (detail.components.length === 0)
    return (
      <EmptyState title="No components yet">
        Ask an agent to add them from the project's code, so mockups use its real buttons, rows and cards.
      </EmptyState>
    )
  return (
    <div className="grid gap-5 md:grid-cols-[180px_minmax(0,1fr)]">
      <nav aria-label="components" className="flex flex-col gap-px self-start max-md:hidden md:sticky md:top-16">
        {detail.components.map((component) => (
          <a key={component.name} href={`#component-${component.name}`} className="rounded-sm px-2.5 py-1 font-mono text-xs text-fg-2 hover:bg-surface-2 hover:text-fg-1">
            {component.name}
          </a>
        ))}
      </nav>
      <div className="flex min-w-0 flex-col gap-3">
        {detail.components.map((component) => (
          <ComponentCard key={component.name} component={component} css={detail.css} theme={theme} phone={phone} />
        ))}
      </div>
    </div>
  )
}

const SPECIMEN = 'Sphinx of black quartz, judge my vow'

function Tokens({ detail, theme }: { detail: DesignDetail; theme: 'light' | 'dark' }) {
  const [box, width] = useWidth<HTMLDivElement>()
  const themes = detail.themes.slice(0, 2)
  const colors = detail.tokens.filter((token) => token.family === 'color')
  const others = detail.tokens.filter((token) => token.family !== 'color')
  const families = [...new Set(others.map((token) => token.family))]
  const value = (values: Record<string, string>, id: string) => values[id] ?? values['*'] ?? Object.values(values)[0] ?? ''
  const specimen = detail.type_styles
    .map((style) => `<div style="display:grid;grid-template-columns:130px 1fr;gap:12px;align-items:baseline;padding:6px 0;border-bottom:1px solid var(--border-subtle,#8883)"><div style="font:11px/1.4 ui-monospace,monospace;opacity:.75">${escapeHtml(style.name)}<br>${escapeHtml(style.font_size)}${style.font_weight ? ` · ${escapeHtml(style.font_weight)}` : ''}</div><div class="ds-${escapeHtml(style.name)}" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${SPECIMEN}</div></div>`)
    .join('')
  return (
    <div className="grid gap-7 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
      <section aria-label="colours" className="min-w-0">
        <p className="mb-1 flex justify-between font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">
          <span>Colour · {themes.map((each) => each.name).join(' | ') || 'one theme'}</span>
          <span>{colors.length}</span>
        </p>
        {colors.map((token) => (
          <div key={token.name} className="grid grid-cols-[72px_minmax(0,140px)_minmax(0,1fr)] items-center gap-3 border-b border-line-subtle py-1.5">
            <span className="flex h-7 overflow-hidden rounded-sm border border-line-subtle">
              {(themes.length > 0 ? themes : [{ id: '*', name: '' }]).map((each) => (
                <span key={each.id} className="flex-1" style={{ background: value(token.values, each.id) }} />
              ))}
            </span>
            <span className="min-w-0">
              <span className="block truncate font-mono text-xs text-fg-1">{token.name}</span>
              <span className="block truncate font-mono text-2xs text-fg-3">{(themes.length > 0 ? themes : [{ id: '*', name: '' }]).map((each) => value(token.values, each.id)).join(' · ')}</span>
            </span>
            <span className="text-xs text-fg-2">{token.usage}</span>
          </div>
        ))}
      </section>
      <section aria-label="type and shape" className="flex min-w-0 flex-col gap-5">
        {detail.type_styles.length > 0 && (
          <div ref={box}>
            <p className="mb-1 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">Type</p>
            <SystemFrame html={`<div style="padding:0 2px">${specimen}</div>`} css={detail.css} theme={theme} width={width} title="type scale" />
          </div>
        )}
        {families.map((family) => (
          <div key={family}>
            <p className="mb-1.5 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">{family}</p>
            {others
              .filter((token) => token.family === family)
              .map((token) => (
                <div key={token.name} className="flex items-center gap-2.5 py-0.5" title={token.usage}>
                  <span className="w-28 shrink-0 truncate font-mono text-xs text-fg-1">{token.name}</span>
                  <TokenShape family={family} value={value(token.values, theme)} />
                  <span className="min-w-0 truncate font-mono text-2xs text-fg-3">{value(token.values, theme)}</span>
                </div>
              ))}
          </div>
        ))}
      </section>
    </div>
  )
}

/** A spacing step as a bar, a radius as a corner, a shadow on a card; anything else as its value alone. */
function TokenShape({ family, value }: { family: string; value: string }) {
  const px = /^\d+(\.\d+)?px$/.test(value) ? Number.parseFloat(value) : null
  if (family === 'spacing' && px !== null) return <span className="h-2.5 shrink-0 border border-accent bg-accent-tint" style={{ width: Math.min(px * 2, 160) }} />
  if (family === 'radius') return <span className="h-6 w-10 shrink-0 border border-fg-2 bg-surface-1" style={{ borderRadius: value }} />
  if (family === 'shadow') return <span className="h-6 w-10 shrink-0 rounded-sm bg-elevated" style={{ boxShadow: value }} />
  return null
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function Asset({ search, path, mediaType }: { search: DesignSearch; path: string; mediaType: string }) {
  const asset = useQuery({
    queryKey: keys.designAsset(search, path),
    queryFn: () => api.designAsset(search, path),
    enabled: mediaType.startsWith('image/'),
    staleTime: Infinity,
  })
  return (
    <figure className="m-0 flex min-w-0 flex-col gap-1">
      <span className="flex h-24 items-center justify-center rounded-sm border border-line-subtle bg-surface-1 p-2">
        {asset.data ? <img src={`data:${asset.data.media_type};base64,${asset.data.data}`} alt={path} className="max-h-full max-w-full" /> : <Icon name="file" className="text-fg-3" />}
      </span>
      <figcaption className="truncate font-mono text-2xs text-fg-2">{path.split('/').pop()}</figcaption>
    </figure>
  )
}

function Files({ detail, search }: { detail: DesignDetail; search: DesignSearch }) {
  const groups = [...new Set(detail.assets.filter((asset) => !asset.path.endsWith('README.md')).map((asset) => asset.path.split('/')[0]))]
  return (
    <div className="flex flex-col gap-6">
      {groups.map((group) => (
        <section key={group} aria-label={group}>
          <p className="mb-2 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">{group}</p>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-3">
            {detail.assets
              .filter((asset) => asset.path.startsWith(`${group}/`) && !asset.path.endsWith('README.md'))
              .map((asset) => (
                <Asset key={asset.path} search={search} path={asset.path} mediaType={asset.media_type} />
              ))}
          </div>
        </section>
      ))}
      <section aria-label="files">
        <p className="mb-1.5 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">
          Files <span className="normal-case">· {detail.summary.dir}</span>
        </p>
        <ul className="m-0 list-none rounded-md border border-line bg-elevated p-0">
          {detail.files.map((file) => (
            <li key={file} className="border-b border-line-subtle px-3 py-1 font-mono text-xs text-fg-1 last:border-b-0">
              {file}
            </li>
          ))}
        </ul>
      </section>
      {detail.dropped.length > 0 && (
        <section aria-label="left out">
          <p className="mb-1.5 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">Left out</p>
          <ul className="m-0 flex flex-col gap-1 pl-4 text-xs text-fg-2">
            {detail.dropped.map((why) => (
              <li key={why}>{why}</li>
            ))}
          </ul>
        </section>
      )}
    </div>
  )
}

function Cover({ detail, theme }: { detail: DesignDetail; theme: 'light' | 'dark' }) {
  const [box, width] = useWidth<HTMLDivElement>()
  if (!detail.cover) return null
  return (
    <div ref={box} className="overflow-hidden rounded-md border border-line">
      <SystemFrame html={detail.cover} css={detail.css} theme={theme} width={960} scale={width > 0 ? Math.min(1, width / 960) : 1} least={200} title={`${detail.summary.title} cover`} />
    </div>
  )
}

type SystemTab = 'components' | 'tokens' | 'rules' | 'files'

/** One system's page: its cover, components drawn live, tokens, rules and files. */
export function DesignSystemPage() {
  const search = useSearch({ strict: false }) as DesignSearch
  const page = useQuery({ queryKey: keys.designSystem(search), queryFn: () => api.designSystem(search), refetchInterval: search.thread ? 4000 : false })
  const shown = useShownTheme()
  const [tab, setTab] = useState<SystemTab>('components')
  const [theme, setTheme] = useState<'light' | 'dark' | null>(null)
  const [width, setWidth] = useState<'desktop' | 'phone'>('desktop')
  const [changing, setChanging] = useState(false)
  const detail = page.data
  const where = detail ? (detail.summary.source === 'personal' ? 'yours' : search.clone?.split('/').pop() ?? 'repository') : ''
  return (
    <Page
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Link to="/settings/$section" params={{ section: 'design-systems' }} className="text-fg-2 hover:text-fg-1">
            Design systems
          </Link>
          <Icon name="chevronRight" className="shrink-0 text-fg-3" />
          <span className="truncate">{detail?.summary.title ?? search.name}</span>
        </span>
      }
      meta={detail ? `${where} · ${detail.summary.dir}` : undefined}
      actions={
        detail && (
          <>
            {detail.summary.uses.map((use) => (
              <Tag key={use} tone="accent">
                {use}
              </Tag>
            ))}
            {detail.summary.thread && (
              <Link to="/t/$id" params={{ id: detail.summary.thread }} className="text-xs text-accent-fg hover:text-fg-1">
                {detail.summary.unmerged ? 'Being made in a thread' : 'Being changed in a thread'}
              </Link>
            )}
            <Button icon="agent" onClick={() => setChanging(true)} disabled={detail.summary.unmerged}>
              Change it with an agent
            </Button>
          </>
        )
      }
      label="design system"
      width="wide"
    >
      <div className="flex flex-col gap-4 px-4 py-4 md:px-6">
        {page.isError && <p className="text-sm text-danger">{message(page.error)}</p>}
        {detail && (
          <>
            <Cover detail={detail} theme={theme ?? shown} />
            <div className="flex flex-wrap items-center justify-between gap-2">
              <Tabs
                label="design system"
                tabs={[
                  { id: 'components' as const, label: <>Components <span className="font-mono text-2xs text-fg-3">{detail.components.length}</span></> },
                  { id: 'tokens' as const, label: 'Tokens' },
                  { id: 'rules' as const, label: 'Rules' },
                  { id: 'files' as const, label: 'Files' },
                ]}
                value={tab}
                onChange={setTab}
              />
              <span className="flex flex-wrap items-center gap-2">
                {tab === 'components' && <Segmented label="width" value={width} options={[{ id: 'desktop', label: 'Desktop' }, { id: 'phone', label: 'Phone' }]} onChange={setWidth} />}
                {(tab === 'components' || tab === 'tokens') && (
                  <Segmented label="theme" value={theme ?? shown} options={[{ id: 'light', label: 'Light' }, { id: 'dark', label: 'Dark' }]} onChange={setTheme} />
                )}
              </span>
            </div>
            {tab === 'components' && <Components detail={detail} theme={theme ?? shown} phone={width === 'phone'} />}
            {tab === 'tokens' && <Tokens detail={detail} theme={theme ?? shown} />}
            {tab === 'rules' && (detail.readme.trim() ? <Markdown text={detail.readme} className="max-w-[72ch] text-[14px]" /> : <EmptyState title="No rules yet">Its README.md is where an agent learns how to use it.</EmptyState>)}
            {tab === 'files' && <Files detail={detail} search={search} />}
            {detail.made_from && <p className="font-mono text-2xs text-fg-3">made from {detail.made_from}</p>}
            <ChangeDesignSystem open={changing} onClose={() => setChanging(false)} detail={detail} search={search} />
          </>
        )}
      </div>
    </Page>
  )
}

/** The thread's panel while its agent makes or changes a system: each file as it's written, and the latest component. */
export function ThreadDesignPanel({ thread, working }: { thread: string; working: boolean }) {
  const design = useQuery({ queryKey: keys.threadDesign(thread), queryFn: () => api.threadDesign(thread), refetchInterval: working ? 3000 : false })
  const shown = useShownTheme()
  const [box, width] = useWidth<HTMLDivElement>()
  const made = design.data
  if (!made) return null
  const detail = made.detail
  const latest = detail?.components.at(-1)
  const search: DesignSearch = detail
    ? { source: detail.summary.source, name: detail.summary.name, ...(detail.summary.source === 'project' ? { thread } : {}) }
    : { source: made.keep === 'personal' ? 'personal' : 'project', name: made.name }
  const state = working ? (made.changing ? 'changing' : 'building') : made.changing ? 'changed' : 'built'
  return (
    <div className="flex flex-col gap-3 px-4 py-4">
      <div className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate font-medium text-fg-1">{made.title}</span>
          <span className="font-mono text-2xs text-fg-3">
            {state}
            {detail && ` · ${detail.components.length} components`}
          </span>
        </span>
        {detail && (
          <Link to="/settings/design-systems/system" search={search} className="inline-flex items-center gap-1 text-xs text-accent-fg hover:text-fg-1">
            Open <Icon name="chevronRight" />
          </Link>
        )}
      </div>
      {!detail && <p className="text-sm text-fg-2">Nothing written yet. Its files show here as the agent writes them.</p>}
      {detail && (
        <>
          <span className="flex flex-wrap gap-1">
            {detail.tokens
              .filter((token) => token.family === 'color')
              .slice(0, 14)
              .map((token) => (
                <span key={token.name} title={token.name} className="h-[18px] w-[18px] rounded-sm border border-line-subtle" style={{ background: token.values[detail.themes[0]?.id ?? '*'] ?? token.values['*'] }} />
              ))}
          </span>
          <ul className="m-0 list-none rounded-md border border-line bg-elevated px-3 py-1">
            {detail.files.map((file) => (
              <li key={file} className="flex items-center gap-2 border-b border-line-subtle py-1 last:border-b-0">
                <Icon name="check" className="shrink-0 text-ok" />
                <span className="truncate font-mono text-xs text-fg-1">{file}</span>
              </li>
            ))}
          </ul>
          {latest && (
            <div ref={box} className="overflow-hidden rounded-md border border-line bg-elevated">
              <p className="flex items-center gap-2 border-b border-line-subtle px-3 py-1.5">
                <span className="font-mono text-xs text-fg-1">{latest.name}</span>
                <span className="font-mono text-2xs text-fg-3">latest</span>
              </p>
              <SystemFrame html={`${PREVIEW_PAD}${latest.html}</div>`} css={detail.css} theme={shown} width={Math.max(width, 1) / 0.7} scale={0.7} title={`${latest.name} preview`} />
            </div>
          )}
        </>
      )}
    </div>
  )
}
