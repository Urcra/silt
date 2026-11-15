// What agents showed the person (ADR 0068): artifacts, each a run of
// versions under one id, and images shown in the conversation.

import type { ArtifactContent, ArtifactListed, ArtifactView, CommentView, HistoryEntry } from './api'
import {
  FIRST_SCRIPT_BRIDGE,
  FIRST_SCRIPT_END,
  FIRST_SCRIPT_START,
  FONT_SOURCES,
  OFFLINE_STUB,
  SCRIPT_SOURCES,
  STYLE_SOURCES,
  VENDOR_HOSTS,
} from './frame.generated'

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

/**
 * The policy a frame runs under, as `silt_design::frame::policy` writes it: the CDNs themselves, or only `vendor`, an
 * absolute URL ending in `/` that serves them through Silt (ADR 0071).
 */
export function framePolicy(vendor: string | null): string {
  const scripts = vendor ?? SCRIPT_SOURCES.join(' ')
  const styles = vendor ?? STYLE_SOURCES.join(' ')
  const fonts = vendor ?? FONT_SOURCES.join(' ')
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

/** Whether `origin` is a bare `http(s)://host[:port]`, as `silt_design::frame::is_origin` checks. */
function isOrigin(origin: string): boolean {
  const host = origin.startsWith('https://') ? origin.slice(8) : origin.startsWith('http://') ? origin.slice(7) : null
  return host !== null && host.length > 0 && host.length <= 255 && /^[A-Za-z0-9.:[\]-]+$/.test(host)
}

/**
 * The script that runs first in an artifact's frame, as `silt_design::frame::first_script` writes it. It takes the
 * viewer's theme, now and as it changes, and tells the page holding the frame, at `parent`, what text is selected, for
 * a comment to quote, and what went wrong in it (ADR 0071).
 */
function frameScript(theme: FrameTheme | null, parent: string | null, boot: string | null): string {
  const themed = theme ? `root.setAttribute('data-theme','${theme}');` : ''
  const to = parent !== null && isOrigin(parent) ? parent : '*'
  return `${FIRST_SCRIPT_START}${themed}var to='${to}';var boot=${bootJson(boot)};${FIRST_SCRIPT_END}${FIRST_SCRIPT_BRIDGE}})();`
}

/** The most of a viewer's storage and kept state a frame is handed, as `silt_design::frame::BOOT_BYTES`. */
const BOOT_BYTES = 2 << 20

/** `boot` as a JSON object safe inside a script element, or `{}`, as `silt_design::frame` writes it: checked, then kept as written. */
function bootJson(boot: string | null): string {
  if (boot === null || boot.length > BOOT_BYTES) return '{}'
  let parsed: unknown
  try {
    parsed = JSON.parse(boot)
  } catch {
    return '{}'
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return '{}'
  return boot.replace(/<\//g, '<\\/').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029')
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
  /** The origin of the page holding the frame, which the frame's messages go to. */
  parent?: string | null
  /** The policy as a `<meta>`; a page served with it as a header leaves it out. */
  policyMeta?: boolean
  /** What the holding page kept for this viewer, as a JSON object: their storage, and the last version's state. */
  boot?: string | null
}

/** A page's doctype, or the standard one when it has none, and the rest, as `silt_design::frame::split_doctype` splits it. */
function splitDoctype(html: string): [string, string] {
  const trimmed = html.trimStart()
  const end = trimmed.slice(0, 9).toLowerCase() === '<!doctype' ? trimmed.indexOf('>') : -1
  return end >= 0 ? [trimmed.slice(0, end + 1), trimmed.slice(end + 1)] : ['<!doctype html>', trimmed]
}

/**
 * An HTML artifact as its sandboxed frame loads it, composed as `silt_design::frame::compose` does on every other
 * surface: a policy that lets it reach nothing but the CDN allowlist, the frame's script and the design system's tokens
 * before anything of its own, and its doctype kept first so it renders as written.
 */
export function sandboxedDocument(
  html: string,
  { probe = true, style = null, theme = null, vendor = null, parent = null, policyMeta = true, boot = null }: FrameOptions = {},
): string {
  if (vendor) {
    html = throughVendor(html, vendor)
    style = style && throughVendor(style, vendor)
  }
  const [doctype, rest] = splitDoctype(html)
  const policy = policyMeta ? `<meta http-equiv="Content-Security-Policy" content="${framePolicy(vendor)}">` : ''
  const script = probe ? `<script>${frameScript(theme, parent, boot)}</script>` : `<script>${OFFLINE_STUB}</script>`
  // The CSS can't close its own element, whatever a peer put in it.
  const styled = style ? `<style data-silt="design-system">${style.replace(/<\//g, '<\\/')}</style>` : ''
  return `${doctype}${policy}${script}${styled}${rest}`
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
 * A version as one HTML file that opens anywhere, with no Silt and no network: an HTML artifact as it is, or as
 * `rendered` with its diagrams drawn in, under the same policy as in Silt; anything else, as `rendered` (Markdown's markup, a diagram's SVG as an image) or the
 * content inline, on a plain page titled with the artifact's name.
 */
/** The path that opens one artifact, at its latest version unless `version` names another. */
export function artifactPath(thread: string, artifact: string, version?: number | null): string {
  return version ? `/t/${thread}/artifacts/${artifact}/v/${version}` : `/t/${thread}/artifacts/${artifact}`
}

/** Whether `row` is among what the filters ask for. */
export function listingMatches(row: ArtifactListed, filters: { text: string; kind: string; repo: string; mine: boolean; archived: boolean }): boolean {
  if (row.archived !== filters.archived) return false
  if (filters.kind && row.artifact.kind !== filters.kind) return false
  if (filters.repo && row.repo !== filters.repo) return false
  if (filters.mine && !row.mine) return false
  const text = filters.text.trim().toLowerCase()
  return !text || `${row.artifact.title} ${row.artifact.description ?? ''}`.toLowerCase().includes(text)
}

/** A version's files as the offline copy carries them: each one's type and its bytes as base64, by path. */
export type InlineFiles = Record<string, { media_type: string; data: string }>

/**
 * `html` with its own files carried inside it, for the offline copy (ADR 0074): each `src` or `href` naming one becomes
 * a `data:` URI, and `fetch` of one answers from what's carried.
 */
export function withFilesInlined(html: string, files: InlineFiles): string {
  const paths = Object.keys(files)
  if (paths.length === 0) return html
  let out = html
  for (const path of paths) {
    const escaped = path.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    const uri = `data:${files[path].media_type.split(';')[0]};base64,${files[path].data}`
    out = out.replace(new RegExp(`((?:src|href)\\s*=\\s*["'])(?:\\./)?${escaped}(["'])`, 'g'), (_match, before: string, after: string) => `${before}${uri}${after}`)
  }
  const carried = JSON.stringify(Object.fromEntries(paths.map((path) => [path, [files[path].media_type, files[path].data]]))).replace(/</g, '\\u003c')
  const shim = `<script>(function(){var f=${carried};var of=window.fetch;window.fetch=function(u,o){var k=String(u&&u.url||u).replace(/^\\.\\//,'');if(Object.prototype.hasOwnProperty.call(f,k)){var b=atob(f[k][1]),a=new Uint8Array(b.length);for(var i=0;i<b.length;i++)a[i]=b.charCodeAt(i);return Promise.resolve(new Response(a,{headers:{'content-type':f[k][0]}}))}return of.apply(this,arguments)}})()</script>`
  const doctype = /^\s*<!doctype[^>]*>/i.exec(out)
  return doctype ? doctype[0] + shim + out.slice(doctype[0].length) : shim + out
}

export function offlineHtml(artifact: ArtifactView, content: ArtifactContent, rendered?: string, style?: string | null, files: InlineFiles = {}): string {
  if (artifact.kind === 'html') return sandboxedDocument(withFilesInlined(rendered ?? textOf(content), files), { probe: false, style })
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
