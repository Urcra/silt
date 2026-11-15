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

/**
 * What an artifact's page may do: run its own inline scripts and styles, load scripts from a few CDNs and fonts from
 * Google Fonts, show what it carries inline, and connect to nothing (ADR 0071).
 */
export const ARTIFACT_CSP = framePolicy(null)

/** The hosts Silt's vendor route fetches from, as `silt_design::frame` lists them. */
const VENDOR_HOSTS = ['cdnjs.cloudflare.com', 'cdn.jsdelivr.net', 'unpkg.com', 'cdn.tailwindcss.com', 'code.jquery.com', 'fonts.googleapis.com', 'fonts.gstatic.com']

/**
 * The policy a frame runs under, as `silt_design::frame::policy` writes it: the CDNs themselves, or only `vendor`, an
 * absolute URL ending in `/` that serves them through Silt (ADR 0071).
 */
export function framePolicy(vendor: string | null): string {
  const scripts = vendor ?? 'https://cdnjs.cloudflare.com https://cdn.jsdelivr.net/npm/ https://unpkg.com https://cdn.tailwindcss.com https://code.jquery.com'
  const styles = vendor ?? 'https://fonts.googleapis.com'
  const fonts = vendor ?? 'https://fonts.gstatic.com'
  return `default-src 'none'; script-src 'unsafe-inline' ${scripts}; style-src 'unsafe-inline' ${styles}; font-src data: ${fonts}; img-src data: blob:; media-src data: blob:`
}

/** `text` with every allowlisted CDN address going through `vendor`, as `silt_design::frame::through_vendor` does. */
export function throughVendor(text: string, vendor: string): string {
  let out = text
  for (const host of VENDOR_HOSTS) {
    out = out.split(`https://${host}/`).join(`${vendor}${host}/`)
    out = out.split(`//${host}/`).join(`${vendor}${host}/`)
  }
  return out.split(`https:${vendor}`).join(vendor).split(`http:${vendor}`).join(vendor)
}

export type FrameTheme = 'light' | 'dark'

/** What the page in an artifact's frame reported going wrong in it: a load the policy blocked, a script error, or a page wider than its frame. */
export type FrameProblem = { kind: 'blocked' | 'error' | 'overflow'; text: string }

/**
 * The script that runs first in an artifact's frame. It takes the viewer's theme, now and as it changes, and tells the
 * page holding the frame what text is selected, for a comment to quote, and what went wrong in it (ADR 0071).
 */
function frameScript(theme: FrameTheme | null): string {
  return [
    '(function(){var root=document.documentElement;',
    theme ? `root.setAttribute('data-theme','${theme}');` : '',
    "addEventListener('message',function(e){var d=e.data;if(e.source!==parent||!d||d.silt!=='theme')return;if(d.theme==='light'||d.theme==='dark')root.setAttribute('data-theme',d.theme)});",
    "document.addEventListener('mouseup',function(){var t=String(getSelection()||'').trim();parent.postMessage({silt:'selection',text:t.slice(0,500)},'*')});",
    'var seen={},problems=[];',
    "function report(kind,text){text=String(text).slice(0,300);if(seen[kind+text]||problems.length>=20)return;seen[kind+text]=1;problems.push({kind:kind,text:text});parent.postMessage({silt:'problems',problems:problems.slice()},'*')}",
    "document.addEventListener('securitypolicyviolation',function(e){report('blocked',(e.blockedURI||'inline')+' ('+e.effectiveDirective+')')});",
    "addEventListener('error',function(e){if(e.message)report('error',e.message+(e.lineno?' (line '+e.lineno+')':''))});",
    "addEventListener('unhandledrejection',function(e){var r=e.reason;report('error','Unhandled rejection: '+(r&&r.message||r))});",
    "addEventListener('load',function(){setTimeout(function(){var w=root.scrollWidth,v=root.clientWidth;if(w>v+1)report('overflow','The page is '+w+'px wide in a '+v+'px frame')},300)});",
    '})();',
  ].join('')
}

export type FrameOptions = {
  /** The script that reports selections and problems and follows the theme; an offline copy has none. */
  probe?: boolean
  /** The CSS the version's design system compiled to. */
  style?: string | null
  /** The viewer's theme, set on the page's root. */
  theme?: FrameTheme | null
  /** Silt's vendor route, an absolute URL ending in `/`, when libraries and fonts load through it. */
  vendor?: string | null
}

/**
 * An HTML artifact as its sandboxed frame loads it: with a policy that lets it reach nothing but the CDN allowlist,
 * the frame's script and the design system's tokens before anything of its own, and its doctype kept first so it
 * renders as written.
 */
export function sandboxedDocument(html: string, { probe = true, style = null, theme = null, vendor = null }: FrameOptions = {}): string {
  if (vendor) {
    html = throughVendor(html, vendor)
    style = style && throughVendor(style, vendor)
  }
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html)
  const first = doctype ? doctype[0].trim() : '<!doctype html>'
  const rest = doctype ? html.slice(doctype[0].length) : html
  const policy = `<meta http-equiv="Content-Security-Policy" content="${framePolicy(vendor)}">`
  const script = probe ? `<script>${frameScript(theme)}</script>` : ''
  // The CSS can't close its own element, whatever a peer put in it.
  const styled = style ? `<style data-silt="design-system">${style.replace(/<\//g, '<\\/')}</style>` : ''
  return `${first}${policy}${script}${styled}${rest}`
}

/** A prompt asking the agent to fix what the viewer's frame reported in a version. */
export function problemsPrompt(shown: ArtifactVersion, problems: FrameProblem[]): string {
  const { id, title, version } = shown.artifact
  const words = { blocked: 'Blocked', error: 'Script error', overflow: 'Too wide' }
  const items = problems.map((problem) => `- ${words[problem.kind]}: ${problem.text}`)
  return [
    `The viewer's frame saw problems in the artifact "${title}" (id ${id}, version ${version}):`,
    '',
    ...items,
    '',
    `Fix them in a new version with silt_artifact and id ${id}. Only the CDNs and Google Fonts load; inline everything else.`,
  ].join('\n')
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
export function offlineHtml(artifact: ArtifactView, content: ArtifactContent, rendered?: string, style?: string | null): string {
  if (artifact.kind === 'html') return sandboxedDocument(textOf(content), { probe: false, style })
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
