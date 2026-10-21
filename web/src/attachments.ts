import type { PromptAttachment } from './api'

export function readAttachment(file: File): Promise<PromptAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error(`could not read ${file.name}`))
    reader.onload = () => {
      const url = String(reader.result)
      resolve({ name: file.name, mime: file.type || 'application/octet-stream', data: url.slice(url.indexOf(',') + 1) })
    }
    reader.readAsDataURL(file)
  })
}
