import { useQuery } from '@tanstack/react-query'
import { type KeyboardEvent, useState } from 'react'
import { api, type Subfolder } from './api'
import { keys } from './query'

const SHOWN = 50

/** A typed path as the folder it's in, kept as typed, and the start of the name after it; none before a separator's typed. */
export function splitTyped(typed: string): { dir: string; partial: string } | null {
  const at = Math.max(typed.lastIndexOf('/'), typed.lastIndexOf('\\'))
  return at < 0 ? null : { dir: typed.slice(0, at + 1), partial: typed.slice(at + 1) }
}

/** The folders whose names start with `partial`, as typed in any case; hidden ones only once a dot's typed. */
export function completions(folders: Subfolder[], partial: string): Subfolder[] {
  const typed = partial.toLowerCase()
  return folders.filter((folder) => folder.name.toLowerCase().startsWith(typed) && (typed.startsWith('.') || !folder.name.startsWith('.')))
}

/** How far every name agrees, in any case, spelt as the first one is. */
export function sharedStart(names: string[]): string {
  const [first = '', ...rest] = names
  let end = first.length
  for (const name of rest) {
    let at = 0
    while (at < end && at < name.length && name[at].toLowerCase() === first[at].toLowerCase()) at++
    end = at
  }
  return first.slice(0, end)
}

/**
 * Completes a typed path's last part from the folders the daemon lists, as a shell does: Tab completes what the
 * matches share, then the one picked; the arrows pick, and Enter takes the picked one.
 */
export function usePathCompletion(typed: string, setTyped: (path: string) => void) {
  const split = splitTyped(typed)
  const dir = split?.dir ?? ''
  const listed = useQuery({ queryKey: keys.subfolders(dir), queryFn: () => api.subfolders(dir), enabled: split !== null, staleTime: 30_000, retry: false })
  const matches = split && listed.data ? completions(listed.data, split.partial).slice(0, SHOWN) : []
  // Picked for what's typed now, so typing on forgets it.
  const [picked, setPicked] = useState<{ typed: string; index: number }>()
  const active = picked?.typed === typed && picked.index < matches.length ? picked.index : -1
  const pick = (index: number) => setPicked({ typed, index })
  const complete = (index: number) => setTyped(`${dir}${matches[index].name}${dir.slice(-1)}`)
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (!split || matches.length === 0) return
    const count = matches.length
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const step = event.key === 'ArrowDown' ? 1 : -1
      pick(active < 0 ? (step > 0 ? 0 : count - 1) : (active + step + count) % count)
    } else if (event.key === 'Tab' && !event.shiftKey) {
      event.preventDefault()
      const shared = sharedStart(matches.map((match) => match.name))
      if (active >= 0) complete(active)
      else if (count === 1) complete(0)
      else if (shared.length > split.partial.length) setTyped(`${dir}${shared}`)
      else pick(0)
    } else if (event.key === 'Enter' && active >= 0) {
      event.preventDefault()
      complete(active)
    }
  }
  return { dir, matches, active, complete, onKeyDown }
}

/** A typed path without the separators it ends with, though a root keeps its own. */
export function withoutTrailingSeparator(path: string): string {
  return path.replace(/([^\\/:])[\\/]+$/, '$1')
}
