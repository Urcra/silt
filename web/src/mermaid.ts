// Mermaid, drawn in the page and shown as an image, so nothing in a diagram runs (ADR 0068, ADR 0073).

/** Draws Mermaid as SVG, shown as an image so nothing in it runs. */
export async function mermaidSvg(text: string, id: string, theme: 'neutral' | 'dark' = 'neutral'): Promise<string> {
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme, flowchart: { htmlLabels: false } })
  const { svg } = await mermaid.render(id.replace(/[^a-zA-Z0-9]/g, ''), text)
  return svg
}

/**
 * An HTML page with each `<pre class="mermaid">` drawn into it as an image, for a copy that opens with no Silt to draw
 * them (ADR 0073). A page without diagrams comes back as it was.
 */
export async function withDiagramsDrawn(html: string): Promise<string> {
  if (!/class\s*=\s*["'][^"']*\bmermaid\b/i.test(html)) return html
  const page = new DOMParser().parseFromString(html, 'text/html')
  const diagrams = [...page.querySelectorAll('pre.mermaid')].slice(0, 20)
  if (diagrams.length === 0) return html
  for (const [at, diagram] of diagrams.entries()) {
    try {
      const svg = await mermaidSvg(diagram.textContent ?? '', `offline-${at}`)
      const image = page.createElement('img')
      image.src = `data:image/svg+xml;base64,${btoa(String.fromCharCode(...new TextEncoder().encode(svg)))}`
      image.alt = diagram.getAttribute('aria-label') ?? 'Diagram'
      image.style.maxWidth = '100%'
      diagram.replaceWith(image)
    } catch {
      // A diagram that doesn't parse stays as its text.
    }
  }
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html)?.[0].trim() ?? '<!doctype html>'
  return `${doctype}${page.documentElement.outerHTML}`
}
