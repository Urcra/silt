// What Settings › Machines and the toasts share about machines over SSH (ADR 0108).

import type { ReactNode } from 'react'
import type { MachineFields } from './desktop'

/** `text` with each `quoted` part as code, as the desktop app writes commands and hosts in its reasons. */
export function withCode(text: string): ReactNode {
  return text.split('`').map((part, at) =>
    at % 2 === 1 ? (
      <code key={at} className="font-mono">
        {part}
      </code>
    ) : (
      part
    ),
  )
}

/** The same checks the desktop app makes, so the form says what it would refuse. */
export function machineProblem(fields: { name: string; destination: string; port: string }, taken: string[]): string | undefined {
  const name = fields.name
  if (!name) return 'Give it a name'
  if (name.trim() !== name || [...name].length > 64 || !/^[\p{L}\p{N}._\- ]+$/u.test(name)) {
    return 'A name has letters, digits, spaces, dots, dashes or underscores'
  }
  if (taken.includes(name)) return 'Another machine has this name'
  const [user, host] = fields.destination.includes('@') ? fields.destination.split(/@(.*)/s) : [undefined, fields.destination]
  const plain = (part: string | undefined, extra = '') =>
    part !== undefined &&
    part !== '' &&
    !part.startsWith('-') &&
    [...part].every((char) => /[A-Za-z0-9._-]/.test(char) || extra.includes(char))
  if (fields.destination.length > 255 || !plain(host, ':') || (user !== undefined && !plain(user)))
    return 'Write the destination as user@host, or a Host from ~/.ssh/config'
  if (fields.port !== '') {
    const port = Number(fields.port)
    if (!/^\d+$/.test(fields.port) || port < 1 || port > 65535) return 'A port is a number from 1 to 65535'
  }
  return undefined
}

export function fieldsOf(form: { name: string; destination: string; port: string }): MachineFields {
  return { name: form.name, destination: form.destination, port: form.port === '' ? null : Number(form.port) }
}
