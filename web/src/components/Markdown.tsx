import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useHighlighted } from '../highlight'
import { COMPONENTS } from './markdownParts'
import { cx } from '../ui'

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
