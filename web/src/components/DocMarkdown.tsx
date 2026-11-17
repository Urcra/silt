// A doc's text as its read view shows it (ADR 0077), apart from chat's Markdown so a thread's messages load none of it.

import { type ReactElement, type ReactNode, Suspense, lazy, useEffect, useId, useState } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { svgUrl } from '../artifacts'
import { mermaidSvg } from '../mermaid'
import { useShownTheme } from '../theme'
import { cx } from '../ui'
import { COMPONENTS, docMarkdown, textOf } from './markdownParts'

// A doc's charts load with the data page's viewer, only when a doc has one.
const DocChart = lazy(() => import('./TypedArtifact').then((viewer) => ({ default: viewer.DocChart })))

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
        <time dateTime={day} className="rounded-sm bg-surface-1 px-1" data-md={`<?silt date ${day}?>`} contentEditable={false}>
          {Number.isNaN(date.getTime()) ? day : date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })}
        </time>
      )
    }
    if (href.startsWith('#silt-mention:')) {
      const name = href.slice('#silt-mention:'.length)
      return (
        <span className="rounded-sm bg-accent/15 px-1 text-accent-fg" data-md={`<?silt mention ${name}?>`} contentEditable={false}>
          @{name}
        </span>
      )
    }
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
