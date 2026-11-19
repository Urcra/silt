import type { Block, Card, Effect } from '../api'
import { Tag, cx } from '../ui'

const NOTE: Record<string, string> = {
  neutral: 'border-line bg-bg text-fg-2',
  good: 'border-ok bg-ok-tint text-fg-1',
  warn: 'border-warn bg-warn-tint text-fg-1',
  danger: 'border-danger bg-danger-tint text-fg-1',
}

const LABEL = 'font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase'

/** One of a card's blocks, drawn by Silt from what the plugin prepared (ADR 0104). */
function CardBlock({ block }: { block: Block }) {
  switch (block.type) {
    case 'fields':
      return (
        <dl className="grid grid-cols-[minmax(80px,max-content)_minmax(0,1fr)] gap-x-3 gap-y-1 text-sm">
          {block.pairs.map(([name, value]) => (
            <div key={name} className="contents">
              <dt className="text-fg-2">{name}</dt>
              <dd className="min-w-0 font-mono break-words text-fg-1">{value}</dd>
            </div>
          ))}
        </dl>
      )
    case 'list':
      return (
        <div className="flex flex-col gap-1">
          <span className={LABEL}>{block.label}</span>
          <ul className="flex flex-col gap-0.5 text-sm">
            {block.items.map((item, index) => (
              <li key={index} className="flex min-w-0 gap-2">
                {item.key && <code className="shrink-0 font-mono text-xs text-fg-2">{item.key}</code>}
                <span className="min-w-0 break-words">{item.text}</span>
              </li>
            ))}
          </ul>
        </div>
      )
    case 'diff':
      return (
        <div className="flex flex-col gap-1">
          <span className={LABEL}>{block.label}</span>
          <ul className="flex flex-col gap-0.5 font-mono text-xs">
            {block.files.map((file) => (
              <li key={file.path} className="flex min-w-0 gap-2">
                <span className="min-w-0 truncate">{file.path}</span>
                <span className="ml-auto shrink-0 text-ok">+{file.added}</span>
                <span className="shrink-0 text-danger">−{file.removed}</span>
              </li>
            ))}
          </ul>
        </div>
      )
    case 'code':
      return (
        <div className="flex flex-col gap-1">
          <span className={LABEL}>{block.label}</span>
          <pre className="max-h-64 overflow-y-auto rounded-sm border border-line bg-bg px-2.5 py-2 font-mono text-xs whitespace-pre-wrap text-fg-1 [overflow-wrap:anywhere]">
            {block.text}
          </pre>
        </div>
      )
    case 'note':
      return <p className={cx('rounded-sm border px-2.5 py-1.5 text-xs', NOTE[block.tone])}>{block.text}</p>
  }
}

/** What Silt lets through while the call runs, in words. */
function effectLine(effect: Effect): { text: string; checked: boolean } {
  switch (effect.type) {
    case 'http':
      return {
        text: `${effect.method} ${effect.url}${effect.body_sha256 ? ` · body ${effect.body_sha256.slice(0, 4)}…${effect.body_sha256.slice(-4)}` : ''}`,
        checked: true,
      }
    case 'git_push':
      return {
        text: `push ${effect.new.slice(0, 7)} to ${effect.reference.replace('refs/heads/', '')} of ${effect.remote}, only if it's still at ${effect.old ? effect.old.slice(0, 7) : 'nothing'}`,
        checked: true,
      }
    case 'reach':
      return { text: `${effect.path} into the agent's sandbox, ${effect.writable ? 'writable' : 'read-only'}`, checked: true }
    case 'opaque':
      return { text: effect.what, checked: false }
  }
}

/** A WebAssembly plugin's call as Silt draws it: the plugin's card, and every effect Silt lets through. */
export function PluginCard({ card, effects, why }: { card: Card; effects: Effect[]; why?: string | null }) {
  return (
    <div className="flex flex-col gap-2.5">
      <span className={LABEL}>Prepared by the plugin, drawn by Silt</span>
      <div className="flex flex-col gap-2.5 rounded-md border border-line bg-bg px-3 py-2.5" aria-label="the call">
        <span className="text-sm font-medium text-fg-1">{card.title}</span>
        {card.blocks.map((block, index) => (
          <CardBlock key={index} block={block} />
        ))}
        {effects.length > 0 && (
          <div className="flex flex-col gap-1">
            <span className={LABEL}>Silt lets through only this</span>
            {effects.map((effect, index) => {
              const { text, checked } = effectLine(effect)
              return (
                <span key={index} className="flex min-w-0 items-baseline gap-2 font-mono text-xs [overflow-wrap:anywhere]">
                  {!checked && <Tag tone="warn">Silt can't check</Tag>}
                  {text}
                </span>
              )
            })}
          </div>
        )}
      </div>
      {why && <span className="text-2xs text-fg-2">Asked by {why}.</span>}
    </div>
  )
}
