/** A background task's report that Claude Code puts in the conversation as the user's turn, which nobody typed. */
export interface TaskNotification {
  summary: string
  status: string
  result: string
}

export function taskNotification(text: string): TaskNotification | null {
  const body = /^\s*<task-notification>([\s\S]*?)(<\/task-notification>\s*)?$/.exec(text)?.[1]
  if (body === undefined) return null
  const field = (name: string) => new RegExp(`<${name}>([\\s\\S]*?)</${name}>`).exec(body)?.[1].trim() ?? ''
  return { summary: field('summary') || 'A background task reported back', status: field('status'), result: field('result') }
}
