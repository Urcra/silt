import { describe, expect, it, vi } from 'vitest'
import { api, type ArtifactView } from '../api'
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

function holder(agrees: boolean, opened: string[] = [], added: string[] = []): Holder {
  return {
    confirm: async () => agrees,
    openComposer: (text) => opened.push(text),
    addComment: async (text, quote) => {
      added.push(quote ? `${quote}: ${text}` : text)
      return `c${added.length}`
    },
    commentIds: () => ['c1'],
    commentsChanged: () => undefined,
  }
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

  it('comments as the viewer once they agree, and only replies to comments on this artifact', async () => {
    const added: string[] = []
    const broker = new Broker({ thread: 't', artifact, emit: () => undefined }, handlersFor('t', artifact, holder(true, [], added)))
    const made = await broker.answer({ id: 1, cap: 'comments', method: 'add', args: { text: 'Move this up', quote: 'Q3 row' } })
    expect(made).toEqual({ ok: true, value: { id: 'c1' } })
    expect(added).toEqual(['Q3 row: Move this up'])
    const foreign = await broker.answer({ id: 2, cap: 'comments', method: 'reply', args: { id: 'elsewhere', text: 'hi' } })
    expect(foreign).toMatchObject({ ok: false, error: { code: 'refused' } })
    const declined = new Broker({ thread: 't', artifact, emit: () => undefined }, handlersFor('t', artifact, holder(false)))
    expect(await declined.answer({ id: 3, cap: 'comments', method: 'add', args: { text: 'no' } })).toMatchObject({ ok: false, error: { code: 'not_granted' } })
    const composerOnly: ArtifactView = { ...artifact, capabilities: { comments: { composer_only: true } } }
    const opener = new Broker({ thread: 't', artifact: composerOnly, emit: () => undefined }, handlersFor('t', composerOnly, holder(true)))
    expect(await opener.answer({ id: 4, cap: 'comments', method: 'add', args: { text: 'no' } })).toMatchObject({ ok: false })
  })
})

describe('the built-in permissions', () => {
  it('is served to every page, declared or not', async () => {
    const plain: ArtifactView = { ...artifact, capabilities: null }
    const broker = new Broker({ thread: 't', artifact: plain, emit: () => undefined }, handlersFor('t', plain, holder(true)))
    expect(await broker.answer({ id: 1, cap: 'silt', method: 'use', args: { name: 'permissions' } })).toMatchObject({ ok: true, value: { available: true } })
    expect(await broker.answer({ id: 2, cap: 'silt', method: 'use', args: { name: 'sample' } })).toMatchObject({ ok: true, value: { available: false } })
  })
  it('says a capability that never asks is granted wherever it is served', async () => {
    vi.spyOn(api, 'artifactGrants').mockResolvedValue({})
    const page: ArtifactView = { ...artifact, capabilities: { db: {}, sample: {}, dictation: {} } }
    const broker = new Broker({ thread: 't', artifact: page, emit: () => undefined }, handlersFor('t', page, holder(true)))
    const answered = await broker.answer({ id: 1, cap: 'permissions', method: 'state', args: {} })
    expect(answered).toMatchObject({ ok: true, value: { db: 'granted', sample: 'prompt' } })
  })
})

describe('the people a page sees', () => {
  it('names them only when the page declares the profile scope', async () => {
    vi.spyOn(api, 'pageViewer').mockResolvedValue({ id: 'me1', level: 'owner', people: { me1: 'Ada', p2: 'Grace' } })
    const plain: ArtifactView = { ...artifact, capabilities: { user: {} } }
    const named: ArtifactView = { ...artifact, capabilities: { user: { scopes: ['profile'] } } }
    const ask = (view: ArtifactView, id: number, method: string, args: unknown) =>
      new Broker({ thread: 't', artifact: view, emit: () => undefined }, handlersFor('t', view, holder(true))).answer({ id, cap: 'user', method, args })
    expect(await ask(plain, 1, 'profiles', { ids: ['p2'] })).toEqual({ ok: true, value: { p2: { id: 'p2', name: '' } } })
    expect(await ask(plain, 2, 'search', { query: 'gr' })).toMatchObject({ ok: false })
    expect(await ask(named, 3, 'profiles', { ids: ['p2'] })).toEqual({ ok: true, value: { p2: { id: 'p2', name: 'Grace' } } })
    expect(await ask(named, 4, 'search', { query: 'gr' })).toEqual({ ok: true, value: [{ id: 'p2', name: 'Grace' }] })
  })
})
