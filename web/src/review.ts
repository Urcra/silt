import type { CommentView } from './api'

/** Where a line comment sits, as the daemon anchors it. */
export interface Lines {
  path: string
  start: number
  end: number
}

export function linesOf(comment: CommentView): Lines | null {
  return comment.location.at === 'lines' ? comment.location : null
}

/** The prompt asking the agent to address the comments, one per line. */
export function reviewPrompt(comments: CommentView[]): string {
  const items = comments.flatMap((comment) => {
    const at = linesOf(comment)
    if (!at) return []
    const lines = at.end !== at.start ? `${at.start}-${at.end}` : `${at.start}`
    return [`- ${at.path}:${lines} (${comment.by}): ${comment.text}`]
  })
  return `Please address these review comments:\n${items.join('\n')}`
}
