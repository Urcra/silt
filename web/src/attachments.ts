import { useState } from 'react'
import type { PromptAttachment } from './api'

export function readAttachment(file: File, name = file.name): Promise<PromptAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error(`could not read ${name}`))
    reader.onload = () => {
      const url = String(reader.result)
      resolve({ name, mime: file.type || 'application/octet-stream', data: url.slice(url.indexOf(',') + 1) })
    }
    reader.readAsDataURL(file)
  })
}

/** A pasted file's name: a screenshot on the clipboard is `image.png` however many are pasted, so each gets a number. */
export function pastedName(file: { name: string; type: string }, nth: number): string {
  if (file.name && !/^image\.\w+$/.test(file.name)) return file.name
  const extension = file.type.split('/')[1]?.replace('jpeg', 'jpg').replace(/\+.*/, '') || 'bin'
  return `pasted-${nth}.${extension}`
}

/** The files pasted or dropped; some browsers list a pasted screenshot only among the items. */
export function filesFrom(data: DataTransfer | null): File[] {
  if (!data) return []
  if (data.files.length > 0) return [...data.files]
  return [...data.items].filter((item) => item.kind === 'file').flatMap((item) => item.getAsFile() ?? [])
}

/** The largest file a prompt takes, and all of a prompt's files together, as the daemon's `acp::decode` has them. */
export const FILE_BYTES = 10 << 20
export const FILES_BYTES = 25 << 20

/** Why `files` can't be added to those `attached`, if they can't: refused before they're read and sent. */
export function tooLarge(files: { name: string; size: number }[], attached: number): string | null {
  const large = files.find((file) => file.size > FILE_BYTES)
  if (large) return `${large.name} is too large: a prompt takes files of up to 10 MiB`
  const total = files.reduce((sum, file) => sum + file.size, attached)
  return total > FILES_BYTES ? 'These files are too large together: a prompt takes up to 25 MiB of files' : null
}

/**
 * The files a prompt carries. `add` names pasted ones apart. Any file goes: one the agent can't read in the
 * prompt itself, such as a spreadsheet, the daemon saves where the agent can read it.
 */
export function useAttachments() {
  const [attached, setAttached] = useState<PromptAttachment[]>([])
  const add = async (files: File[], pasted = false) => {
    const refused = tooLarge(
      files,
      attached.reduce((sum, file) => sum + (file.data.length * 3) / 4, 0),
    )
    if (refused) throw new Error(refused)
    const named = files.map((file, index) => ({ file, name: pasted ? pastedName(file, attached.length + index + 1) : file.name }))
    const read = await Promise.all(named.map(({ file, name }) => readAttachment(file, name)))
    setAttached((before) => [...before, ...read])
  }
  const remove = (index: number) => setAttached((before) => before.filter((_, kept) => kept !== index))
  const restore = (files: PromptAttachment[]) => setAttached((before) => [...files, ...before])
  return { attached, add, remove, restore, clear: () => setAttached([]) }
}

/** What the composer's + offers to attach. */
export const ATTACH = { label: 'Attach files or images' }
