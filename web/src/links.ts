// Read-only links (ADR 0027, ADR 0073).

/** The link's token, from a path such as `/r/<token>`. */
export function linkToken(path: string): string | null {
  return /^\/r\/([A-Za-z0-9_-]{16,128})\/?$/.exec(path)?.[1] ?? null
}
