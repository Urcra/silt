// What chat's Markdown and a doc's read view share: how each piece of Markdown is drawn, and a doc's blocks as Markdown.

import { type ReactElement, type ReactNode, isValidElement } from 'react'
import { type Components } from 'react-markdown'
import { languageOf } from '../highlight'
import { cx } from '../ui'
import { Code } from './Markdown'

export function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children)
  return ''
}

export const COMPONENTS: Components = {
  pre({ children }) {
    const code = (Array.isArray(children) ? children[0] : children) as ReactElement<{ className?: string; children?: ReactNode }>
    const language = /language-([\w+-]+)/.exec(code?.props?.className ?? '')?.[1] ?? ''
    const text = textOf(code?.props?.children).replace(/\n$/, '')
    return (
      <div className="my-2 overflow-hidden rounded-sm border border-line-subtle bg-surface-1">
        {language && <div className="border-b border-line-subtle px-2.5 py-1 font-mono text-2xs text-fg-2">{language}</div>}
        <pre className="overflow-x-auto px-2.5 py-2 font-mono text-xs leading-5">
          <Code code={text} language={languageOf(language)} />
        </pre>
      </div>
    )
  },
  code({ children }) {
    // A short chip, a path or a name, stays whole on its line; a long one wraps with each piece boxed.
    const short = textOf(children).length <= 40
    return (
      <code
        className={cx(
          'rounded-sm border border-line-subtle bg-surface-1 px-1 font-mono text-[0.92em]',
          short ? 'whitespace-nowrap' : '[box-decoration-break:clone]',
        )}
      >
        {children}
      </code>
    )
  },
  a({ href, children }) {
    const outside = href !== undefined && /^https?:/.test(href)
    return (
      <a
        href={href}
        className="text-accent-fg underline decoration-line-strong underline-offset-2 hover:text-fg-1"
        {...(outside ? { target: '_blank', rel: 'noreferrer noopener' } : {})}
      >
        {children}
      </a>
    )
  },
  h1: ({ children }) => <h3 className="mt-3 mb-1 text-lg font-medium">{children}</h3>,
  h2: ({ children }) => <h4 className="mt-3 mb-1 text-base font-medium">{children}</h4>,
  h3: ({ children }) => <h5 className="mt-2 mb-1 text-sm font-semibold">{children}</h5>,
  h4: ({ children }) => <h6 className="mt-2 mb-1 text-sm font-semibold">{children}</h6>,
  p: ({ children }) => <p className="my-1.5">{children}</p>,
  ul: ({ children }) => <ul className="my-1.5 list-disc pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="my-1.5 list-decimal pl-5">{children}</ol>,
  li: ({ children }) => <li className="my-0.5">{children}</li>,
  blockquote: ({ children }) => <blockquote className="my-2 border-l-2 border-line-strong pl-3 text-fg-2">{children}</blockquote>,
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="border-collapse text-xs">{children}</table>
    </div>
  ),
  th: ({ children }) => <th className="border border-line bg-surface-1 px-2 py-1 text-left font-medium">{children}</th>,
  td: ({ children }) => <td className="border border-line px-2 py-1 align-top">{children}</td>,
  hr: () => <hr className="my-3 border-line" />,
  img: ({ alt }) => <span className="text-fg-2">[image{alt ? `: ${alt}` : ''}]</span>,
}

/**
 * A doc's blocks (ADR 0077) as Markdown the doc's view draws: `<?silt date 2026-11-15?>` and `<?silt mention ann?>`
 * as links it shows as a date and a name, `<?silt chart {…}?>` as a fenced block it draws as a chart. One that isn't
 * well formed is left as it was, which Markdown then drops as HTML.
 */
export function docMarkdown(text: string): string {
  return text.replace(/<\?silt\s+(date|mention|chart)\s+([\s\S]*?)\?>/g, (all, kind: string, body: string) => {
    const value = body.trim()
    if (kind === 'date') return /^\d{4}-\d{2}-\d{2}$/.test(value) ? `[${value}](#silt-date:${value})` : all
    if (kind === 'mention') return /^[\w.@-]{1,64}$/.test(value) ? `[@${value}](#silt-mention:${value})` : all
    return `\n\n\`\`\`silt-chart\n${value.replace(/`/g, '')}\n\`\`\`\n\n`
  })
}
