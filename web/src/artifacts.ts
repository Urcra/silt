// What agents showed the person (ADR 0068): artifacts, each a run of
// versions under one id, and images shown in the conversation.

import type { ArtifactContent, ArtifactView, CommentView, HistoryEntry } from './api'

export type ArtifactVersion = { entry: HistoryEntry; artifact: ArtifactView }

/** One artifact: its versions, oldest first. */
export type ArtifactGroup = { id: string; versions: ArtifactVersion[] }

/** The thread's artifacts, the one changed last first; images shown in the conversation aren't among them. */
export function artifactsOf(entries: HistoryEntry[]): ArtifactGroup[] {
  const groups = new Map<string, ArtifactGroup>()
  for (const entry of entries) {
    const artifact = entry.artifact
    if (!artifact || artifact.inline) continue
    const group = groups.get(artifact.id) ?? { id: artifact.id, versions: [] }
    group.versions.push({ entry, artifact })
    groups.set(artifact.id, group)
  }
  for (const group of groups.values()) group.versions.sort((a, b) => a.artifact.version - b.artifact.version)
  return [...groups.values()].sort((a, b) => latest(b).entry.index - latest(a).entry.index)
}

/** The images agents showed in the conversation, newest first. */
export function shownImagesOf(entries: HistoryEntry[]): ArtifactVersion[] {
  return entries.flatMap((entry) => (entry.artifact?.inline ? [{ entry, artifact: entry.artifact }] : [])).reverse()
}

export function latest(group: ArtifactGroup): ArtifactVersion {
  return group.versions[group.versions.length - 1]
}

/** Version `version` of the group, or its latest when it has no such version. */
export function versionOf(group: ArtifactGroup, version: number | null): ArtifactVersion {
  return group.versions.find((candidate) => candidate.artifact.version === version) ?? latest(group)
}

const KINDS: Record<string, string> = { html: 'HTML page', svg: 'SVG', markdown: 'Document', mermaid: 'Diagram', image: 'Image' }

export function kindLabel(kind: string): string {
  return KINDS[kind] ?? kind
}

/** What an artifact's page may do: run its own inline scripts and styles, show what it carries inline, and reach nothing. */
export const ARTIFACT_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:"

/** Tells the page holding the frame what text is selected in it, for a comment to quote. */
const SELECTION_SCRIPT =
  "document.addEventListener('mouseup',function(){var t=String(getSelection()||'').trim();parent.postMessage({silt:'selection',text:t.slice(0,500)},'*')})"

/**
 * An HTML artifact as its sandboxed frame loads it: with a policy that lets it reach nothing, before anything of its
 * own, and its doctype kept first so it renders as written. `selection` adds the script that reports selected text.
 */
export function sandboxedDocument(html: string, selection = true): string {
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html)
  const first = doctype ? doctype[0].trim() : '<!doctype html>'
  const rest = doctype ? html.slice(doctype[0].length) : html
  const policy = `<meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}">`
  return `${first}${policy}${selection ? `<script>${SELECTION_SCRIPT}</script>` : ''}${rest}`
}

/** An artifact's content as text. */
export function textOf(content: ArtifactContent): string {
  const binary = atob(content.data)
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0))
  return new TextDecoder().decode(bytes)
}

export function dataUrl(content: ArtifactContent): string {
  return `data:${content.media_type};base64,${content.data}`
}

/** SVG markup as an image's address, so no script in it runs. */
export function svgUrl(svg: string): string {
  const bytes = new TextEncoder().encode(svg)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return `data:image/svg+xml;base64,${btoa(binary)}`
}

const EXTENSIONS: Record<string, string> = {
  'text/html': 'html',
  'image/svg+xml': 'svg',
  'text/markdown': 'md',
  'text/vnd.mermaid': 'mmd',
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/webp': 'webp',
}

/** A file name for a version: its title as a slug, its version and its type's extension. */
export function fileName(artifact: ArtifactView, extension = EXTENSIONS[artifact.media_type] ?? 'bin'): string {
  const slug = artifact.title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  return `${slug || 'artifact'}-v${artifact.version}.${extension}`
}

function escape(text: string): string {
  return text.replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char] ?? char)
}

/**
 * A version as one HTML file that opens anywhere, with no Silt and no network: an HTML artifact as it is, under the
 * same policy as in Silt; anything else, as `rendered` (Markdown's markup, a diagram's SVG as an image) or the
 * content inline, on a plain page titled with the artifact's name.
 */
export function offlineHtml(artifact: ArtifactView, content: ArtifactContent, rendered?: string): string {
  if (artifact.kind === 'html') return sandboxedDocument(textOf(content), false)
  const title = escape(`${artifact.title} · v${artifact.version}`)
  const body = rendered ?? `<img alt="${escape(artifact.title)}" src="${dataUrl(content)}">`
  return [
    '<!doctype html>',
    `<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">`,
    `<meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}">`,
    `<title>${title}</title>`,
    '<style>body{margin:0 auto;max-width:880px;padding:32px 24px;font:15px/1.6 system-ui,sans-serif;color:#1d1d1f;background:#fff}',
    'img,svg{max-width:100%;height:auto}pre,code{font-family:ui-monospace,monospace;font-size:13px}pre{overflow:auto;background:#f5f5f5;padding:12px;border-radius:6px}',
    'table{border-collapse:collapse}td,th{border:1px solid #ddd;padding:4px 8px}</style></head>',
    `<body>${body}</body></html>`,
  ].join('')
}

/** The comments made on any version of `group`: those on one of its versions' entries. */
export function commentsOn(group: ArtifactGroup, comments: CommentView[]): CommentView[] {
  const indices = new Set(group.versions.map((version) => version.entry.index))
  return comments.filter((comment) => comment.location.at === 'entry' && indices.has(comment.location.index))
}

/** A comment's words with the text it's about quoted first. */
export function quoted(text: string, quote: string | null): string {
  if (!quote) return text.trim()
  const lines = quote.trim().split('\n').map((line) => `> ${line}`)
  return `${lines.join('\n')}\n\n${text.trim()}`
}

/** The prompt that sends comments on an artifact to its agent, asking for a new version. */
export function artifactPrompt(shown: ArtifactVersion, comments: CommentView[]): string {
  const { id, title, version } = shown.artifact
  const items = comments.map((comment) => `- ${comment.text.replace(/\n/g, '\n  ')}`)
  return [`Comments on the artifact "${title}" (id ${id}, version ${version}):`, '', ...items, '', `Make a new version with silt_artifact and id ${id}.`].join('\n')
}
