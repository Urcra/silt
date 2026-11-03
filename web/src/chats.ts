// Chats: threads that belong to no repository, each in a folder of its own (ADR 0059).

/** The project every chat belongs to. */
export const CHATS = 'silt:chats'

export function isChat(repo: string | null | undefined): boolean {
  return repo === CHATS
}
