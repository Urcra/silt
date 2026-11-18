// Settings › Appearance's themes, and a theme's page (ADR 0098).

import { useQuery } from '@tanstack/react-query'
import { Link, useSearch } from '@tanstack/react-router'
import { useRef, useState } from 'react'
import { type AppearanceSettings, type ThemeSummary, api } from '../api'
import { keys, message } from '../query'
import { hosted } from '../served'
import { SETTINGS_HEADING } from '../settingsContext'
import { THEMES, useAppearance } from '../theme'
import { Button, Icon, Tag, cx, toast } from '../ui'
import { Page } from './Page'

const LABEL = 'font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase'

type Filter = 'all' | 'light' | 'dark' | 'yours'

const FILTERS: { id: Filter; label: string; shows: (theme: ThemeSummary) => boolean }[] = [
  { id: 'all', label: 'All', shows: () => true },
  { id: 'light', label: 'Light', shows: (theme) => theme.appearance === 'light' },
  { id: 'dark', label: 'Dark', shows: (theme) => theme.appearance === 'dark' },
  { id: 'yours', label: 'Yours', shows: (theme) => theme.source !== 'bundled' },
]

function from(theme: ThemeSummary): string {
  const where = theme.source === 'bundled' ? 'bundled' : theme.source === 'yours' ? 'yours' : "yours, over Silt's"
  return [theme.appearance, where, theme.license === 'AGPL-3.0-only' ? null : theme.license].filter(Boolean).join(' · ')
}

/** bg, surface-1, fg-1 and accent, as small squares. */
function Swatch({ colors, size = 'md' }: { colors: string[]; size?: 'sm' | 'md' }) {
  const box = size === 'sm' ? 'h-3 w-3' : 'h-4 w-4'
  return (
    <span aria-hidden="true" className="flex shrink-0 gap-0.5">
      {colors.slice(0, 4).map((color, index) => (
        <span key={index} className={cx(box, index === 0 && 'border border-line-subtle')} style={{ background: color }} />
      ))}
    </span>
  )
}

function withSlot(settings: AppearanceSettings, theme: ThemeSummary): AppearanceSettings {
  return theme.appearance === 'light' ? { ...settings, light: theme.id } : { ...settings, dark: theme.id }
}

/** The mode, the light and dark theme, and the list to pick from. */
export function ThemeSettings() {
  const { catalog, settings, save } = useAppearance()
  const [filter, setFilter] = useState<Filter>('all')
  const file = useRef<HTMLInputElement>(null)
  const themes = Array.isArray(catalog.data?.themes) ? catalog.data.themes : []
  const choose = (next: AppearanceSettings) => save(next).catch((failure: unknown) => toast(message(failure), 'danger'))
  const yours = themes.filter((theme) => theme.source !== 'bundled').length
  const shown = themes.filter(FILTERS.find((each) => each.id === filter)!.shows)
  const importFile = async (picked: File | undefined) => {
    if (!picked) return
    try {
      const saved = await api.importTheme(await picked.text())
      toast(`Imported ${saved.themes.join(', ')} into ${saved.file}`, 'ok')
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const slot = (appearance: 'light' | 'dark') => {
    const id = appearance === 'light' ? settings.light : settings.dark
    const options = themes.filter((theme) => theme.appearance === appearance)
    const current = options.find((theme) => theme.id === id)
    const showing = settings.mode === appearance
    return (
      <label className="flex min-w-0 flex-col gap-1.5">
        <span className="text-xs text-fg-2">
          {appearance === 'light' ? 'Light theme' : 'Dark theme'}
          {showing && <span className="text-fg-3"> · showing</span>}
          {!current && catalog.data && <span className="text-danger"> · {id} is gone; Nous shows</span>}
        </span>
        <span className="relative flex h-[30px] items-center gap-2 rounded-sm border border-line bg-elevated px-2.5">
          {current && <Swatch colors={current.swatch} size="sm" />}
          <select
            aria-label={appearance === 'light' ? 'light theme' : 'dark theme'}
            value={current?.id ?? ''}
            onChange={(event) => void choose(appearance === 'light' ? { ...settings, light: event.target.value } : { ...settings, dark: event.target.value })}
            className="min-w-0 flex-1 cursor-pointer appearance-none bg-transparent pr-5 text-sm text-fg-1 focus:outline-none"
          >
            {!current && <option value="">Nous</option>}
            {options.map((theme) => (
              <option key={theme.id} value={theme.id}>
                {theme.name}
              </option>
            ))}
          </select>
          <Icon name="chevronDown" className="pointer-events-none absolute right-2 h-3 w-3 text-fg-2" />
        </span>
      </label>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Appearance</h2>
      <div role="radiogroup" aria-label="theme" className="inline-flex self-start overflow-hidden rounded-sm border border-line text-xs">
        {THEMES.map((choice) => (
          <button
            key={choice}
            type="button"
            role="radio"
            aria-checked={settings.mode === choice}
            onClick={() => void choose({ ...settings, mode: choice })}
            className={cx('px-3 py-1 capitalize max-md:min-h-11', settings.mode === choice ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2')}
          >
            {choice}
          </button>
        ))}
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {slot('light')}
        {slot('dark')}
      </div>

      <div className="flex flex-wrap items-center gap-2 pt-3">
        <p className={cx(LABEL, 'flex-1')}>
          Themes · {themes.length - yours} bundled{yours > 0 && ` · ${yours} yours`}
        </p>
        {!hosted() && (
          <>
            <input ref={file} type="file" accept=".json,.jsonc,application/json" className="hidden" aria-label="theme file" onChange={(event) => void importFile(event.target.files?.[0])} />
            <Button icon="download" onClick={() => file.current?.click()}>
              Import from VS Code or Zed
            </Button>
          </>
        )}
      </div>
      <div role="tablist" aria-label="show" className="flex flex-wrap gap-1 text-xs">
        {FILTERS.map((each) => (
          <button
            key={each.id}
            type="button"
            role="tab"
            aria-selected={filter === each.id}
            onClick={() => setFilter(each.id)}
            className={cx('rounded-sm px-2 py-0.5 max-md:min-h-11', filter === each.id ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1')}
          >
            {each.label} <span className="font-mono text-2xs text-fg-3">{themes.filter(each.shows).length}</span>
          </button>
        ))}
      </div>
      {catalog.isError && <p className="text-sm text-danger">{message(catalog.error)}</p>}
      <ul aria-label="themes" className="flex flex-col gap-1.5">
        {catalog.data?.problems.map((problem) => (
          <li key={problem.file} className="flex flex-col gap-0.5 rounded-md border border-danger bg-elevated px-3.5 py-2.5">
            <span className="font-mono text-xs text-fg-1">{problem.file}</span>
            <span className="text-xs text-danger">{problem.message}</span>
          </li>
        ))}
        {shown.map((theme) => {
          const inSlot = (theme.appearance === 'light' ? settings.light : settings.dark) === theme.id
          const trouble = theme.warnings.length > 0
          return (
            <li key={theme.id} className={cx('flex items-center gap-3 rounded-md border bg-elevated px-3.5 py-2.5', trouble ? 'border-danger' : 'border-line')}>
              <Swatch colors={theme.swatch} />
              <Link to="/settings/appearance/theme" search={{ id: theme.id }} className="flex min-w-0 flex-1 flex-col gap-0.5 hover:text-fg-1">
                <span className="flex flex-wrap items-baseline gap-2">
                  <span className="text-sm font-medium text-fg-1">{theme.name}</span>
                  <span className="font-mono text-2xs text-fg-3">{theme.id}</span>
                </span>
                {trouble && (
                  <span className="truncate text-xs text-danger">
                    {theme.file}: {theme.warnings[0]}
                    {theme.warnings.length > 1 && ` (and ${theme.warnings.length - 1} more)`}
                  </span>
                )}
                {theme.failing > 0 && <span className="text-xs text-fg-2">{theme.failing} text pairs below 4.5:1</span>}
              </Link>
              <span className="hidden text-right font-mono text-2xs text-fg-2 sm:block">{from(theme)}</span>
              {inSlot ? (
                <Tag tone="accent">{theme.appearance === 'light' ? 'Light theme' : 'Dark theme'}</Tag>
              ) : (
                <Button size="sm" aria-label={`use ${theme.name} as the ${theme.appearance} theme`} onClick={() => void choose(withSlot(settings, theme))}>
                  Use
                </Button>
              )}
            </li>
          )
        })}
      </ul>
      <div className="rounded-md border border-dashed border-line px-3.5 py-3 text-sm text-fg-2">
        {hosted() ? (
          <>Silt's own themes, and yours from your machines; your pick is kept in this browser.</>
        ) : (
          <>
            Themes you save in <code className="font-mono text-xs">{catalog.data?.folder ?? 'themes/'}</code> show here as you save them, and the one in use
            changes as you edit it. Start from a theme's page with Duplicate to edit, or with <code className="font-mono text-xs">silt theme new</code>.
          </>
        )}
      </div>
    </div>
  )
}

/** One theme's page: its colours, highlight tags, contrast, type and radii. */
export function ThemePage() {
  const { id } = useSearch({ strict: false }) as { id: string }
  const page = useQuery({ queryKey: keys.themeDetail(id), queryFn: () => api.themeDetail(id) })
  const { settings, save } = useAppearance()
  const detail = page.data
  const theme = detail?.summary
  const inUse = theme && (theme.appearance === 'light' ? settings.light : settings.dark) === theme.id
  const duplicate = () =>
    api.duplicateTheme(id).then(
      (saved) => toast(`Saved as ${saved.themes[0]} in ${saved.file}; edit it and Silt shows your changes as you save`, 'ok'),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const failing = detail?.checks.filter((check) => !check.passes && !check.mark).length ?? 0
  return (
    <Page
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Link to="/settings/$section" params={{ section: 'appearance' }} className="text-fg-2 hover:text-fg-1">
            Appearance
          </Link>
          <Icon name="chevronRight" className="shrink-0 text-fg-3" />
          <span className="truncate">{theme?.name ?? id}</span>
          <span className="font-mono text-2xs text-fg-3">{id}</span>
        </span>
      }
      meta={theme ? [from(theme), theme.file, detail?.copyright].filter(Boolean).join(' · ') : undefined}
      actions={
        theme && (
          <>
            {!hosted() && <Button onClick={() => void duplicate()}>Duplicate to edit</Button>}
            {inUse ? (
              <Tag tone="accent">{theme.appearance === 'light' ? 'Light theme' : 'Dark theme'}</Tag>
            ) : (
              <Button variant="primary" onClick={() => void save(withSlot(settings, theme)).catch((failure: unknown) => toast(message(failure), 'danger'))}>
                Use as the {theme.appearance} theme
              </Button>
            )}
          </>
        )
      }
      label="theme"
      width="wide"
    >
      <div className="flex flex-col gap-5 px-4 py-4 md:px-6">
        {page.isError && <p className="text-sm text-danger">{message(page.error)}</p>}
        {detail && (
          <div className="grid gap-7 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
            <div className="flex min-w-0 flex-col gap-6">
              <section aria-label="colours">
                <p className={cx(LABEL, 'mb-1 flex justify-between')}>
                  <span>Colour · {detail.summary.appearance}</span>
                  <span>
                    {detail.colors.length} · {detail.colors.filter((color) => color.derived).length} derived
                  </span>
                </p>
                {detail.colors.map((color) => (
                  <div key={color.token} className="grid grid-cols-[40px_minmax(0,150px)_minmax(0,1fr)] items-center gap-3 border-b border-line-subtle py-1.5">
                    <span className="h-7 rounded-sm border border-line-subtle" style={{ background: color.value }} />
                    <span className="min-w-0">
                      <span className="block truncate font-mono text-xs text-fg-1">{color.token}</span>
                      <span className="block truncate font-mono text-2xs text-fg-3">
                        {color.value}
                        {color.derived && ' · derived'}
                      </span>
                    </span>
                    <span className="text-xs text-fg-2">{color.about}</span>
                  </div>
                ))}
              </section>
              <section aria-label="syntax">
                <p className={cx(LABEL, 'mb-1')}>Syntax</p>
                {detail.syntax.map((tag) => (
                  <div key={tag.tag} className="grid grid-cols-[minmax(0,150px)_minmax(0,1fr)] items-center gap-3 border-b border-line-subtle py-1.5">
                    <span
                      className="truncate font-mono text-xs"
                      style={{ color: tag.color ?? undefined, fontStyle: tag.font_style ?? undefined, fontWeight: tag.font_weight ?? undefined }}
                    >
                      {tag.tag}
                    </span>
                    <span className="truncate text-xs text-fg-2">
                      {[tag.color, tag.font_style, tag.font_weight].filter(Boolean).join(' · ') || 'as the text around it'} · {tag.about}
                    </span>
                  </div>
                ))}
              </section>
            </div>
            <div className="flex min-w-0 flex-col gap-6">
              <section aria-label="contrast">
                <p className={cx(LABEL, 'mb-1 flex justify-between')}>
                  <span>Contrast</span>
                  <span className={failing > 0 ? 'text-danger' : undefined}>{failing > 0 ? `${failing} below 4.5` : 'all text passes'}</span>
                </p>
                {detail.checks.map((check) => (
                  <div key={check.pair} className={cx('flex items-center gap-2.5 border-b border-line-subtle px-1.5 py-1', !check.passes && !check.mark && 'bg-danger-tint')}>
                    <span aria-hidden="true" className="grid h-[18px] w-[26px] shrink-0 place-items-center border border-line-subtle text-[10px] font-semibold" style={{ background: check.back, color: check.fore }}>
                      Aa
                    </span>
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{check.pair}</span>
                    <span className={cx('font-mono text-xs', check.passes ? 'text-fg-2' : check.mark ? 'text-fg-1' : 'text-danger')}>
                      {check.ratio.toFixed(2)} : 1{!check.passes && check.mark && ' · advice'}
                    </span>
                  </div>
                ))}
                {failing > 0 && !hosted() && (
                  <p className="pt-2 text-xs text-fg-2">
                    Silt shows it as written. <code className="font-mono">silt theme check --fix-contrast</code> moves what fails until it reads.
                  </p>
                )}
              </section>
              <section aria-label="type" className="flex flex-col gap-1">
                <p className={LABEL}>Type</p>
                <div className="flex gap-3 border-b border-line-subtle py-1.5 text-sm">
                  <span className="w-14 font-mono text-xs text-fg-2">ui</span>
                  <span>{detail.ui_font}</span>
                </div>
                <div className="flex gap-3 border-b border-line-subtle py-1.5 text-sm">
                  <span className="w-14 font-mono text-xs text-fg-2">mono</span>
                  <span className="font-mono">{detail.mono_font}</span>
                </div>
              </section>
              <section aria-label="radius" className="flex flex-col gap-1.5">
                <p className={LABEL}>Radius</p>
                {['sm', 'md', 'lg'].map((name, index) => (
                  <div key={name} className="flex items-center gap-3">
                    <span className="w-14 font-mono text-xs text-fg-2">{name}</span>
                    <span className="h-[22px] w-[38px] border border-accent bg-accent-tint" style={{ borderRadius: detail.radii[index] }} />
                    <span className="font-mono text-2xs text-fg-3">{detail.radii[index]}px</span>
                  </div>
                ))}
              </section>
            </div>
          </div>
        )}
      </div>
    </Page>
  )
}
