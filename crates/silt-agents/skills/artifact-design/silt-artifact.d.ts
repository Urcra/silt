// The page side of an artifact's bridge in Silt (ADR 0072), as `window.silt`.
// Declare what a page uses with `capabilities` on `silt_artifact`; each one
// resolves to `null` where this view can't run it (not declared, not
// granted, not served here, or failed), so render without it first.

interface SiltRefusal extends Error {
  /** `not_granted`, `rate_limited`, `conflict`, `too_large`, `refused` or `unavailable`. */
  code: string
}

interface SiltHot {
  /** What the last version's snapshot held, when a new version arrived while the page was open; else null. */
  readonly data: unknown
  /** Registers what to keep when a new version arrives: a function returning JSON-able state, at most 1 MiB. */
  snapshot(take: () => unknown): void
  /** Starts the page with the kept state, or `{}`. */
  ready(start: (data: any) => void): void
}

interface Silt {
  readonly version: '1'
  /** A capability's functions, or null. Resolves within 10 s. */
  use(name: string): Promise<any | null>
  readonly hot: SiltHot
}

interface Window {
  readonly silt: Silt
}

// `localStorage` and `sessionStorage` work as usual, kept for this viewer
// and this artifact by the page showing it. They never reach other viewers,
// other machines or the agent.
