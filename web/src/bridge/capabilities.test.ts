import { describe, expect, it } from 'vitest'
import type { ArtifactView } from '../api'
import { Broker } from './broker'
import { downloadName, handlersFor, type Holder } from './capabilities'

const artifact: ArtifactView = {
  id: '0123456789ab',
  version: 3,
  title: 'Planner',
  kind: 'html',
  media_type: 'text/html',
  blob: 'b',
  size: 1,
  inline: false,
  capabilities: { downloads: {}, comments: {}, artifact: {} },
}

function holder(agrees: boolean, opened: string[] = []): Holder {
  return { confirm: async () => agrees, openComposer: (text) => opened.push(text) }
}

describe('the capabilities a page is served', () => {
  it('names downloads without folders and refuses what runs when opened', () => {
    expect(downloadName('../../etc/report.csv')).toBe('_.._etc_report.csv')
    expect(downloadName('plan.xlsx')).toBe('plan.xlsx')
    expect(downloadName('setup.exe')).toBeNull()
    expect(downloadName('run.SH')).toBeNull()
    expect(downloadName('')).toBeNull()
    expect(downloadName('a'.repeat(300))).toHaveLength(128)
  })

  it('saves a download only once the person agrees', async () => {
    const declined = new Broker({ thread: 't', artifact, emit: () => undefined }, handlersFor('t', artifact, holder(false)))
    const answer = await declined.answer({ id: 1, cap: 'downloads', method: 'save', args: { filename: 'a.txt', data: 'hi' } })
    expect(answer).toMatchObject({ ok: false, error: { code: 'not_granted' } })
    const large = await declined.answer({ id: 2, cap: 'downloads', method: 'save', args: { filename: 'a.bin', data: new Uint8Array(17 << 20) } })
    expect(large).toMatchObject({ ok: false, error: { code: 'too_large' } })
  })

  it('opens the composer quoting what the page says, and refuses a publish that is not a document', async () => {
    const opened: string[] = []
    const broker = new Broker({ thread: 't', artifact, emit: () => undefined }, handlersFor('t', artifact, holder(true, opened)))
    await broker.answer({ id: 1, cap: 'comments', method: 'openComposer', args: { text: 'this row' } })
    expect(opened).toEqual(['this row'])
    expect(await broker.answer({ id: 2, cap: 'artifact', method: 'publish', args: { html: '<p>no</p>' } })).toMatchObject({
      ok: false,
      error: { code: 'refused' },
    })
    expect(await broker.answer({ id: 3, cap: 'artifact', method: 'version', args: null })).toEqual({ ok: true, value: { version: 3 } })
  })
})

describe('the built-in permissions', () => {
  it('is served to every page, declared or not', async () => {
    const plain: ArtifactView = { ...artifact, capabilities: null }
    const broker = new Broker({ thread: 't', artifact: plain, emit: () => undefined }, handlersFor('t', plain, holder(true)))
    expect(await broker.answer({ id: 1, cap: 'silt', method: 'use', args: { name: 'permissions' } })).toMatchObject({ ok: true, value: { available: true } })
    expect(await broker.answer({ id: 2, cap: 'silt', method: 'use', args: { name: 'sample' } })).toMatchObject({ ok: true, value: { available: false } })
  })
})
