import { memo, useDeferredValue, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { languageOf, useHighlighted } from '../highlight'
import { COMPONENTS } from './markdownParts'
import { markdownBlocks } from '../markdownBlocks'
import { Icon, cx } from '../ui'

/** Highlighted code, line by line, as it would be in an editor. */
export function Code({ code, language, className }: { code: string; language: string | null; className?: string }) {
  const lines = useHighlighted(code, language)
  return (
    <code className={className}>
      {lines
        ? lines.map((line, index) => (
            <span key={index}>
              {line.map((token, at) =>
                token.className ? (
                  <span key={at} className={token.className}>
                    {token.text}
                  </span>
                ) : (
                  token.text
                ),
              )}
              {index < lines.length - 1 && '\n'}
            </span>
          ))
        : code}
    </code>
  )
}

/** A fenced block, with a button that copies just what it holds. */
export function CodeBlock({ text, language }: { text: string; language: string }) {
  const [copied, setCopied] = useState(false)
  const copy = () =>
    void navigator.clipboard?.writeText(text).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      },
      () => undefined,
    )
  const button = (
    <button
      type="button"
      aria-label={copied ? 'copied' : 'copy code'}
      title="Copy"
      className={cx(
        'rounded-sm p-0.5 text-fg-2 hover:bg-surface-2 hover:text-fg-1',
        !language && 'invisible absolute top-1.5 right-1.5 bg-surface-1 group-hover/code:visible focus:visible max-md:visible',
      )}
      onClick={copy}
    >
      <Icon name={copied ? 'check' : 'copy'} className="h-3.5 w-3.5" />
    </button>
  )
  return (
    <div className="group/code relative my-2 overflow-hidden rounded-sm border border-line-subtle bg-surface-1">
      {language ? (
        <div className="flex items-center justify-between border-b border-line-subtle py-0.5 pr-1.5 pl-2.5 font-mono text-2xs text-fg-2">
          {language}
          {button}
        </div>
      ) : (
        button
      )}
      <pre className="overflow-x-auto px-2.5 py-2 font-mono text-code">
        <Code code={text} language={languageOf(language)} />
      </pre>
    </div>
  )
}

const PROSE = 'min-w-0 leading-(--chat-leading) break-words text-fg-1 [&>:first-child]:mt-0 [&>:last-child]:mb-0'

/** Whether the caller gave a size of its own; else the conversation's, as text and layout set it (ADR 0107). */
const sized = (className?: string) => /(^|\s)text-(2xs|xs|sm|base|lg|ui|ui-lg|label)(\s|$)/.test(className ?? '')

/**
 * A message as Markdown. Its HTML is shown as text and its images by name
 * only: an agent's words are other people's content, which may be hostile.
 */
export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cx(PROSE, !sized(className) && 'text-chat', className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS} skipHtml>
        {text}
      </ReactMarkdown>
    </div>
  )
})

/** One block of a message being streamed, parsed again only when its own text changes. */
const MarkdownBlock = memo(function MarkdownBlock({ text }: { text: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS} skipHtml>
      {text}
    </ReactMarkdown>
  )
})

/**
 * A message as it streams in, as Markdown: block by block, so each word parses only the block it ends, not
 * everything said so far. Parsing the whole reply at each word made a long one fall far behind the agent.
 */
export function StreamingMarkdown({ text, className }: { text: string; className?: string }) {
  // Behind the keys typed and the page scrolled, when there's both to do.
  const shown = useDeferredValue(text)
  const blocks = markdownBlocks(shown)
  return (
    <div className={cx(PROSE, !sized(className) && 'text-chat', className)}>
      {blocks.map((block, index) => (
        // Blocks only grow at the end, so each keeps its place.
        <MarkdownBlock key={index} text={block} />
      ))}
    </div>
  )
}
