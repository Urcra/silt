import { type ReactElement, type ReactNode, Suspense, isValidElement, lazy, useEffect, useId, useState } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { svgUrl } from '../artifacts'
import { languageOf, useHighlighted } from '../highlight'
import { mermaidSvg } from '../mermaid'
import { useShownTheme } from '../theme'
import { cx } from '../ui'

// A doc's charts load with the data page's viewer, only when a doc has one.
const DocChart = lazy(() => import('./TypedArtifact').then((viewer) => ({ default: viewer.DocChart })))

/** Highlighted code, line by line, as it would be in an editor. */
export function Code({ code, language, className }: { code: string; language: string | null; className?: string }) {
  const lines = useHighlighted(code, language)
  return (
    <code className={className}>
      {lines
        ? lines.map((line, index) => (
            <span key={index}>
              {line.map((token, at) => (token.className ? <span key={at} className={token.className}>{token.text}</span> : token.text))}
              {index < lines.length - 1 && '\n'}
            </span>
          ))
        : code}
    </code>
  )
}

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children)
  return ''
}

const COMPONENTS: Components = {
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
      <code className={cx('rounded-sm border border-line-subtle bg-surface-1 px-1 font-mono text-[0.92em]', short ? 'whitespace-nowrap' : '[box-decoration-break:clone]')}>
        {children}
      </code>
    )
  },
  a({ href, children }) {
    const outside = href !== undefined && /^https?:/.test(href)
    return (
      <a href={href} className="text-accent-fg underline decoration-line-strong underline-offset-2 hover:text-fg-1" {...(outside ? { target: '_blank', rel: 'noreferrer noopener' } : {})}>
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

/** A diagram in a doc, drawn as an image; it's `pre.mermaid` until it's drawn, as an offline copy draws those. */
function DocDiagram({ text }: { text: string }) {
  const id = `doc-mermaid${useId()}`
  const theme = useShownTheme()
  const [svg, setSvg] = useState<string | null>(null)
  useEffect(() => {
    let gone = false
    mermaidSvg(text, id, theme === 'dark' ? 'dark' : 'neutral').then(
      (drawn) => !gone && setSvg(drawn),
      () => undefined,
    )
    return () => {
      gone = true
    }
  }, [text, id, theme])
  if (svg) return <img src={svgUrl(svg)} alt="Diagram" className="my-2 max-w-full" data-testid="doc-diagram" />
  return <pre className="mermaid my-2 font-mono text-xs whitespace-pre-wrap text-fg-2">{text}</pre>
}

const DOC_COMPONENTS: Components = {
  ...COMPONENTS,
  pre(props) {
    const code = (Array.isArray(props.children) ? props.children[0] : props.children) as ReactElement<{ className?: string; children?: ReactNode }>
    const language = /language-([\w+-]+)/.exec(code?.props?.className ?? '')?.[1] ?? ''
    const text = textOf(code?.props?.children).replace(/\n$/, '')
    if (language === 'mermaid') return <DocDiagram text={text} />
    if (language === 'silt-chart')
      return (
        <Suspense fallback={<p className="my-2 text-xs text-fg-2">A chart</p>}>
          <DocChart json={text} />
        </Suspense>
      )
    return COMPONENTS.pre ? (COMPONENTS.pre as (given: typeof props) => ReactNode)(props) : null
  },
  a(props) {
    const href = props.href ?? ''
    if (href.startsWith('#silt-date:')) {
      const day = href.slice('#silt-date:'.length)
      const date = new Date(`${day}T00:00:00`)
      return (
        <time dateTime={day} className="rounded-sm bg-surface-1 px-1">
          {Number.isNaN(date.getTime()) ? day : date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })}
        </time>
      )
    }
    if (href.startsWith('#silt-mention:')) return <span className="rounded-sm bg-accent/15 px-1 text-accent-fg">@{href.slice('#silt-mention:'.length)}</span>
    return COMPONENTS.a ? (COMPONENTS.a as (given: typeof props) => ReactNode)(props) : null
  },
}

/** A doc's text as its read view shows it: Markdown with ADR 0077's blocks and Mermaid diagrams drawn. */
export function DocMarkdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cx('min-w-0 text-sm leading-[1.6] break-words text-fg-1 [&>:first-child]:mt-0 [&>:last-child]:mb-0', className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={DOC_COMPONENTS} skipHtml>
        {docMarkdown(text)}
      </ReactMarkdown>
    </div>
  )
}

/**
 * A message as Markdown. Its HTML is shown as text and its images by name
 * only: an agent's words are other people's content (M9's review).
 */
export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cx('min-w-0 text-sm leading-[1.6] break-words text-fg-1 [&>:first-child]:mt-0 [&>:last-child]:mb-0', className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS} skipHtml>
        {text}
      </ReactMarkdown>
    </div>
  )
}
