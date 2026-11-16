// Printing where no headless browser is at hand: the hosted app, which hands a doc or a deck to this browser's own
// print dialog, where it can be saved as a PDF (ADRs 0076, 0077).

import { ARTIFACT_CSP } from './artifacts'

/** `html` under the artifact policy, so what an agent wrote in it reaches nothing while it prints. */
export function printable(html: string): string {
  const policy = `<meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}">`
  return /<head[^>]*>/i.test(html) ? html.replace(/<head[^>]*>/i, (head) => head + policy) : policy + html
}

/**
 * Prints `html` through this browser's print dialog. It loads in a hidden frame of this origin with scripts off, so
 * the holding page can ask it to print and nothing in it runs.
 */
export function printInBrowser(html: string): void {
  const frame = document.createElement('iframe')
  frame.setAttribute('aria-hidden', 'true')
  frame.setAttribute('sandbox', 'allow-same-origin allow-modals')
  frame.style.cssText = 'position:fixed;right:0;bottom:0;width:0;height:0;border:0'
  frame.srcdoc = printable(html)
  frame.onload = () => {
    frame.contentWindow?.focus()
    frame.contentWindow?.print()
    window.setTimeout(() => frame.remove(), 60_000)
  }
  document.body.appendChild(frame)
}
