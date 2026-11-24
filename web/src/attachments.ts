import { useEffect, useRef, useState } from 'react'
import { uploadFile, type PromptAttachment } from './api'
import { message } from './query'
import { hosted } from './served'

export function readAttachment(file: File, name = file.name): Promise<PromptAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error(`could not read ${name}`))
    reader.onload = () => {
      const url = String(reader.result)
      resolve({ name, mime: file.type || 'application/octet-stream', data: url.slice(url.indexOf(',') + 1), upload: null })
    }
    reader.readAsDataURL(file)
  })
}

/** A file attached to a prompt: uploaded to the machine as soon as it's added (ADR 0121). */
export interface Attached extends PromptAttachment {
  /** Stays the same as files come and go. */
  key: string
  size: number
  /** How much has gone, while it uploads. */
  sent: number
  /** Why its upload failed. */
  error?: string
  /** An address in this page to show an image or a video's first frame from. */
  local?: string
}

/** Whether a file is still on its way to the machine. */
export function uploading(file: Attached): boolean {
  return file.upload === null && file.data === '' && !file.error
}

let added = 0

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

/**
 * The largest file a prompt takes in itself, and all of them together, as the daemon's `acp::decode` has them; only
 * where files can't be uploaded first, as the server's app, which takes none (ADR 0121).
 */
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
 * The files a prompt carries. `add` names pasted ones apart and starts uploading each to the machine, with no size
 * limit (ADR 0121); a prompt sent while one is `busy` goes once it's there. Any file goes: one the agent can't read in the
 * prompt itself, such as a spreadsheet or a video, the daemon saves where the agent can read it.
 */
export function useAttachments() {
  const [attached, setAttached] = useState<Attached[]>([])
  const uploads = useRef(new Map<string, AbortController>())
  useEffect(() => {
    const running = uploads.current
    return () => running.forEach((upload) => upload.abort())
  }, [])
  const change = (key: string, update: Partial<Attached>) =>
    setAttached((before) => before.map((file) => (file.key === key ? { ...file, ...update } : file)))
  const add = async (files: File[], pasted = false) => {
    const named = files.map((file, index) => ({ file, name: pasted ? pastedName(file, attached.length + index + 1) : file.name }))
    if (hosted()) {
      const refused = tooLarge(
        files,
        attached.reduce((sum, file) => sum + file.size, 0),
      )
      if (refused) throw new Error(refused)
      const read = await Promise.all(named.map(({ file, name }) => readAttachment(file, name)))
      setAttached((before) => [
        ...before,
        ...read.map((file, index) => ({ ...file, key: `file-${++added}`, size: named[index].file.size, sent: 0 })),
      ])
      return
    }
    const fresh = named.map(({ file, name }): Attached & { file: File } => ({
      key: `file-${++added}`,
      name,
      mime: file.type || 'application/octet-stream',
      data: '',
      upload: null,
      size: file.size,
      sent: 0,
      local: /^(image|video)\//.test(file.type) ? URL.createObjectURL(file) : undefined,
      file,
    }))
    setAttached((before) => [...before, ...fresh.map(({ file: _file, ...kept }) => kept)])
    for (const { key, file } of fresh) {
      const controller = new AbortController()
      uploads.current.set(key, controller)
      uploadFile(file, (sent) => change(key, { sent }), controller.signal)
        .then(
          (done) => change(key, { upload: done.upload, sent: file.size }),
          (failure: unknown) => {
            if (!controller.signal.aborted) change(key, { error: message(failure) })
          },
        )
        .finally(() => uploads.current.delete(key))
    }
  }
  const remove = (index: number) =>
    setAttached((before) => {
      const gone = before[index]
      if (gone) {
        uploads.current.get(gone.key)?.abort()
        if (gone.local) URL.revokeObjectURL(gone.local)
      }
      return before.filter((_, kept) => kept !== index)
    })
  const restore = (files: Attached[]) => setAttached((before) => [...files, ...before])
  const failed = attached.find((file) => file.error)
  return {
    attached,
    add,
    remove,
    restore,
    clear: () => setAttached([]),
    /** Some file is still on its way: a prompt sent now goes once it's there. */
    busy: attached.some(uploading),
    /** Why the prompt can't go with its files, if one didn't upload. */
    failure: failed ? `${failed.name} didn't upload: ${failed.error}; remove it to send` : null,
  }
}

/** What the composer's + offers to attach. */
export const ATTACH = { label: 'Attach files or images' }
