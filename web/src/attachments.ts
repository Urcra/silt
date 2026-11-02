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

/** What an agent takes attached to a prompt: `null` until it's known. */
export type Takes = { agent: string; images: boolean | null; files: boolean | null }

/**
 * The files a prompt carries. `add` names pasted ones apart and refuses images the agent is known not to take,
 * rather than leave the daemon to refuse the prompt.
 */
export function useAttachments(takes: Takes) {
  const [attached, setAttached] = useState<PromptAttachment[]>([])
  const add = async (files: File[], pasted = false) => {
    const named = files.map((file, index) => ({ file, name: pasted ? pastedName(file, attached.length + index + 1) : file.name }))
    const refused = takes.images === false && named.find(({ file }) => file.type.startsWith('image/'))
    if (refused) throw new Error(`${takes.agent} doesn't take files like ${refused.name}`)
    const read = await Promise.all(named.map(({ file, name }) => readAttachment(file, name)))
    setAttached((before) => [...before, ...read])
  }
  const remove = (index: number) => setAttached((before) => before.filter((_, kept) => kept !== index))
  const restore = (files: PromptAttachment[]) => setAttached((before) => [...files, ...before])
  return { attached, add, remove, restore, clear: () => setAttached([]) }
}
