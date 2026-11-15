import { describe, expect, it } from 'vitest'
import type { ArtifactView, CommentView, HistoryEntry } from './api'
import {
  ARTIFACT_CSP,
  artifactPrompt,
  artifactsOf,
  commentsOn,
  fileName,
  framePolicy,
  offlineHtml,
  problemsPrompt,
  quoted,
  sandboxedDocument,
  shownImagesOf,
  textOf,
  throughVendor,
  versionOf,
} from './artifacts'
import { FRAME_FIXTURES, OFFLINE_STUB } from './frame.generated'

function artifact(index: number, id: string, version: number, extra: Partial<ArtifactView> = {}): HistoryEntry {
  return {
    index,
    thread: 't',
    origin: 'own',
    kind: 'artifact',
    text: 'Rescan benchmark',
    time_ms: index,
    rewound: false,
    tool: null,
    outcome: null,
    image: null,
    usage: null,
    plan: null,
    side: null,
    task: null,
    artifact: { id, version, title: 'Rescan benchmark', kind: 'html', media_type: 'text/html', blob: `blob${index}`, size: 10, inline: false, ...extra },
  } as HistoryEntry
}

const base64 = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)))

describe('artifacts', () => {
  it('groups versions by id, the one changed last first, and keeps images shown apart', () => {
    const entries = [
      artifact(1, 'aaaaaaaaaaaa', 1),
      artifact(2, 'bbbbbbbbbbbb', 1, { kind: 'mermaid' }),
      artifact(3, 'cccccccccccc', 1, { kind: 'image', inline: true, media_type: 'image/png' }),
      artifact(4, 'aaaaaaaaaaaa', 2),
    ]
    const groups = artifactsOf(entries)
    expect(groups.map((group) => group.id)).toEqual(['aaaaaaaaaaaa', 'bbbbbbbbbbbb'])
    expect(groups[0].versions.map((version) => version.artifact.version)).toEqual([1, 2])
    expect(versionOf(groups[0], null).artifact.version).toBe(2)
    expect(versionOf(groups[0], 1).artifact.version).toBe(1)
    expect(versionOf(groups[0], 7).artifact.version).toBe(2)
    expect(shownImagesOf(entries).map((image) => image.artifact.id)).toEqual(['cccccccccccc'])
  })

  it('loads an HTML artifact under a policy that allows only the CDN allowlist and connects nowhere, its doctype first', () => {
    const page = sandboxedDocument('<!DOCTYPE html><html><body><script src="https://evil.example/x.js"></script></body></html>')
    expect(page.startsWith('<!DOCTYPE html><meta http-equiv="Content-Security-Policy"')).toBe(true)
    expect(page).toContain(ARTIFACT_CSP)
    const directives = Object.fromEntries(ARTIFACT_CSP.split('; ').map((part) => [part.split(' ')[0], part.split(' ').slice(1)]))
    expect(directives['default-src']).toEqual(["'none'"])
    expect(directives['connect-src']).toBeUndefined()
    expect(directives['script-src']).toEqual([
      "'unsafe-inline'",
      'https://cdnjs.cloudflare.com',
      'https://cdn.jsdelivr.net/npm/',
      'https://unpkg.com',
      'https://cdn.tailwindcss.com',
      'https://code.jquery.com',
    ])
    expect(directives['style-src']).toEqual(["'unsafe-inline'", 'https://fonts.googleapis.com'])
    expect(directives['font-src']).toEqual(['data:', 'https://fonts.gstatic.com'])
    expect(directives['img-src']).toEqual(['data:', 'blob:'])
    expect(sandboxedDocument('<p>hi</p>', { probe: false })).toBe(
      `<!doctype html><meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}"><script>${OFFLINE_STUB}</script><p>hi</p>`,
    )
  })

  it("gives the frame the design system's tokens and the viewer's theme before the page's own content", () => {
    const page = sandboxedDocument('<!doctype html><html lang="en"><p>hi</p>', { style: ':root{--bg:#fff}</style><script>x</script>', theme: 'dark' })
    const policy = page.indexOf('Content-Security-Policy')
    const script = page.indexOf("root.setAttribute('data-theme','dark')")
    const style = page.indexOf('<style data-silt="design-system">:root{--bg:#fff}<\\/style><script>x<\\/script></style>')
    const own = page.indexOf('<html lang="en">')
    expect(policy).toBeGreaterThan(0)
    expect(script).toBeGreaterThan(policy)
    expect(style).toBeGreaterThan(script)
    expect(own).toBeGreaterThan(style)
    expect(page).toContain("d.silt==='theme'")
    expect(page).toContain("'securitypolicyviolation'")
    expect(sandboxedDocument('<p>hi</p>')).not.toContain("root.setAttribute('data-theme','")
  })

  it("loads libraries through Silt's vendor route when that's on, and only from there", () => {
    const vendor = 'http://127.0.0.1:7777/vendor/'
    expect(framePolicy(vendor)).toContain(`script-src 'unsafe-inline' ${vendor};`)
    expect(framePolicy(vendor)).not.toContain('cdnjs')
    expect(throughVendor('<script src="https://cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js"></script><link href="//fonts.googleapis.com/css2?family=Inter">', vendor)).toBe(
      `<script src="${vendor}cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js"></script><link href="${vendor}fonts.googleapis.com/css2?family=Inter">`,
    )
    const page = sandboxedDocument('<script src="https://unpkg.com/react@18.3.1/umd/react.production.min.js"></script>', {
      vendor,
      style: '@import url("https://fonts.googleapis.com/css2?family=Inter");',
    })
    expect(page).toContain(`${vendor}unpkg.com/react@18.3.1/`)
    expect(page).toContain(`@import url("${vendor}fonts.googleapis.com/css2?family=Inter")`)
    expect(page).toContain(framePolicy(vendor))
  })

  it('asks the agent to fix what the frame saw go wrong', () => {
    const entry = artifact(4, 'aaaaaaaaaaaa', 2)
    const prompt = problemsPrompt({ entry, artifact: entry.artifact! }, [
      { kind: 'blocked', text: 'https://esm.sh/react (script-src-elem)' },
      { kind: 'overflow', text: 'The page is 920px wide in a 480px frame' },
    ])
    expect(prompt).toContain('(id aaaaaaaaaaaa, version 2)')
    expect(prompt).toContain('- Blocked: https://esm.sh/react (script-src-elem)')
    expect(prompt).toContain('- Too wide: The page is 920px wide in a 480px frame')
    expect(prompt).toContain('silt_artifact and id aaaaaaaaaaaa')
  })

  it('makes one offline file with nothing outside it', () => {
    const view = artifact(1, 'aaaaaaaaaaaa', 3, { kind: 'image', media_type: 'image/png', title: 'Sidebar <scan>' }).artifact!
    const file = offlineHtml(view, { media_type: 'image/png', data: 'iVBORw0KGgo=' })
    expect(file).toContain('<title>Sidebar &lt;scan&gt; · v3</title>')
    expect(file).toContain('src="data:image/png;base64,iVBORw0KGgo="')
    expect(file).not.toMatch(/(src|href)="https?:/)
    const html = artifact(2, 'bbbbbbbbbbbb', 1).artifact!
    expect(offlineHtml(html, { media_type: 'text/html', data: base64('<h1>é</h1>') })).toContain('<h1>é</h1>')
    expect(offlineHtml(html, { media_type: 'text/html', data: base64('<h1>é</h1>') }, undefined, ':root{--bg:#fff}')).toContain(':root{--bg:#fff}</style><h1>é</h1>')
    expect(fileName(view)).toBe('sidebar-scan-v3.png')
    expect(fileName(html, 'html')).toBe('rescan-benchmark-v1.html')
  })

  it('reads text in UTF-8', () => {
    expect(textOf({ media_type: 'text/markdown', data: base64('# Över') })).toBe('# Över')
  })

  it('keeps comments on an artifact on the version they were made on, and sends them as one prompt', () => {
    const entries = [artifact(1, 'aaaaaaaaaaaa', 1), artifact(4, 'aaaaaaaaaaaa', 2), artifact(6, 'bbbbbbbbbbbb', 1)]
    const group = artifactsOf(entries).find((each) => each.id === 'aaaaaaaaaaaa')!
    const comment = (id: string, index: number, text: string) =>
      ({ id, by: 'alice', time_ms: 0, text, resolved: false, sent: false, location: { at: 'entry', index, excerpt: '' }, replies: [] }) as unknown as CommentView
    const on = commentsOn(group, [comment('c1', 1, 'a'), comment('c2', 4, 'b'), comment('c3', 6, 'c'), comment('c4', 2, 'd')])
    expect(on.map((each) => each.id)).toEqual(['c1', 'c2'])
    expect(quoted('Make it blue', 'the bars\nall of them')).toBe('> the bars\n> all of them\n\nMake it blue')
    expect(quoted(' Plain ', null)).toBe('Plain')
    const prompt = artifactPrompt(group.versions[1], [comment('c2', 4, '> the bars\n\nMake it blue')])
    expect(prompt).toContain('Comments on the artifact "Rescan benchmark" (id aaaaaaaaaaaa, version 2):')
    expect(prompt).toContain('- > the bars\n  \n  Make it blue')
    expect(prompt).toContain('Make a new version with silt_artifact and id aaaaaaaaaaaa.')
  })
})

describe('the frame contract', () => {
  it('composes every frame as silt_design::frame::compose does', () => {
    for (const fixture of FRAME_FIXTURES) {
      const composed = sandboxedDocument(fixture.html, {
        style: fixture.style,
        theme: fixture.theme,
        probe: fixture.probe,
        vendor: fixture.vendor,
        parent: fixture.parent,
        policyMeta: fixture.policyMeta,
        boot: fixture.boot,
      })
      expect(composed).toBe(fixture.out)
    }
  })
})
