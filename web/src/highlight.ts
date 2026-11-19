// Syntax highlighting for code in messages and diffs: Lezer parsers, the same
// CodeMirror's editor uses, each loaded the first time its language shows.

import type { Parser } from '@lezer/common'
import { classHighlighter, highlightCode } from '@lezer/highlight'
import { useEffect, useState } from 'react'

export interface Token {
  text: string
  /** `tok-*` classes from Lezer's class highlighter, styled in index.css. */
  className: string
}

const LANGUAGES: Record<string, () => Promise<Parser>> = {
  rust: async () => (await import('@codemirror/lang-rust')).rustLanguage.parser,
  ts: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser.configure({ dialect: 'ts' }),
  tsx: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser.configure({ dialect: 'ts jsx' }),
  js: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser,
  jsx: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser.configure({ dialect: 'jsx' }),
  python: async () => (await import('@codemirror/lang-python')).pythonLanguage.parser,
  json: async () => (await import('@codemirror/lang-json')).jsonLanguage.parser,
  css: async () => (await import('@codemirror/lang-css')).cssLanguage.parser,
  html: async () => (await import('@codemirror/lang-html')).htmlLanguage.parser,
  markdown: async () => (await import('@codemirror/lang-markdown')).markdownLanguage.parser,
  shell: async () => legacy((await import('@codemirror/legacy-modes/mode/shell')).shell),
  toml: async () => legacy((await import('@codemirror/legacy-modes/mode/toml')).toml),
  yaml: async () => legacy((await import('@codemirror/legacy-modes/mode/yaml')).yaml),
}

async function legacy(mode: Parameters<typeof import('@codemirror/language').StreamLanguage.define>[0]): Promise<Parser> {
  const { StreamLanguage } = await import('@codemirror/language')
  return StreamLanguage.define(mode).parser
}

const ALIASES: Record<string, string> = {
  rs: 'rust',
  typescript: 'ts',
  mts: 'ts',
  cts: 'ts',
  javascript: 'js',
  mjs: 'js',
  cjs: 'js',
  py: 'python',
  htm: 'html',
  md: 'markdown',
  sh: 'shell',
  bash: 'shell',
  zsh: 'shell',
  console: 'shell',
  yml: 'yaml',
  jsonc: 'json',
}

/** The language a fence's info string or a file's name says, if it's one this knows. */
export function languageOf(nameOrInfo: string): string | null {
  const word = nameOrInfo.trim().toLowerCase()
  const extension = word.includes('.') ? word.slice(word.lastIndexOf('.') + 1) : word
  const language = ALIASES[extension] ?? extension
  return language in LANGUAGES ? language : null
}

const loaded = new Map<string, Promise<Parser>>()

function parser(language: string): Promise<Parser> {
  let found = loaded.get(language)
  if (!found) {
    found = LANGUAGES[language]()
    loaded.set(language, found)
  }
  return found
}

/** `code` as lines of highlighted tokens. */
export function tokenize(code: string, using: Parser): Token[][] {
  const lines: Token[][] = [[]]
  highlightCode(
    code,
    using.parse(code),
    classHighlighter,
    (text, className) => lines[lines.length - 1].push({ text, className }),
    () => lines.push([]),
  )
  return lines
}

/** Highlighting waiting its turn, oldest first. */
const waiting: { run: () => void; dropped: boolean }[] = []
let working = false

/** How long highlighting may hold the page before it lets a frame be drawn or a key be read. */
const SLICE_MS = 8

function work() {
  const until = performance.now() + SLICE_MS
  while (waiting.length > 0 && performance.now() < until) {
    const job = waiting.shift()!
    if (!job.dropped) job.run()
  }
  if (waiting.length > 0) setTimeout(work, 0)
  else working = false
}

/** Runs `run` in its turn, a few at a time between frames; answers how to drop it. A large diff's hunks all
 * highlighted at once, as their parser loaded, and the page didn't answer for seconds. */
function inTurn(run: () => void): () => void {
  const job = { run, dropped: false }
  waiting.push(job)
  // The first slice runs now, as one hunk highlighted at once always did; only what's left waits.
  if (!working) {
    working = true
    work()
  }
  return () => {
    job.dropped = true
  }
}

/** `code` highlighted as `language`, once its parser has loaded; null until then, or for a language this doesn't know. */
export function useHighlighted(code: string, language: string | null): Token[][] | null {
  const [done, setDone] = useState<{ code: string; language: string; lines: Token[][] } | null>(null)
  useEffect(() => {
    if (!language) return
    let current = true
    let drop = () => undefined as void
    void parser(language).then((using) => {
      if (!current) return
      drop = inTurn(() => {
        if (current) setDone({ code, language, lines: tokenize(code, using) })
      })
    })
    return () => {
      current = false
      drop()
    }
  }, [code, language])
  return done && done.code === code && done.language === language ? done.lines : null
}
