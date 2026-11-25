// Which calls ask another agent (ADR 0120), read from the call alone.
import type { HistoryEntry } from './api.generated'

/** Silt's own tool a call or a permission names: `silt_ask`, `mcp__silt__silt_ask`, `silt_ask (silt MCP Server)`. */
export function siltTool(title: string | null | undefined): string | null {
  const name = title?.trim().split(/\s+/)[0]?.split('__').pop()
  return name?.startsWith('silt_') ? name : null
}

/** What a `silt_ask` call asked: its input, read from the call. */
export type AskCall = { prompt: string; agent?: string; to?: string; title?: string; model?: string; mode?: string }

export function askOfCall(call: HistoryEntry): AskCall | null {
  if (siltTool(call.tool?.name ?? call.text) !== 'silt_ask') return null
  const at = call.text.indexOf(' {')
  if (at < 0) return null
  try {
    const input = JSON.parse(call.text.slice(at + 1)) as Partial<AskCall>
    return typeof input.prompt === 'string' ? (input as AskCall) : null
  } catch {
    return null
  }
}
