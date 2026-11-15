import { api, type CommentView } from './api'
import { type ArtifactGroup, type ArtifactVersion, artifactPrompt, commentsOn, quoted } from './artifacts'

/** Adds a comment, with what's selected, and sends the artifact's waiting comments to the agent. */
export function useArtifactComments(thread: string, group: ArtifactGroup, shown: ArtifactVersion, comments: CommentView[], onChanged: () => Promise<unknown>) {
  const mine = commentsOn(group, comments)
  const waiting = mine.filter((comment) => !comment.sent && !comment.resolved)
  const add = (text: string, quote: string | null) => api.addComment(thread, { on: 'entry', index: shown.entry.index }, quoted(text, quote))
  const send = async (also: CommentView[] = []) => {
    const all = [...waiting, ...also]
    if (all.length === 0) return
    await api.prompt(thread, artifactPrompt(shown, all))
    await api.commentsSent(thread, all.map((comment) => comment.id)).catch(() => undefined)
    await onChanged()
  }
  return { mine, waiting, add, send }
}
