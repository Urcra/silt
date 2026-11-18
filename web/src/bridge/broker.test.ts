import { describe, expect, it } from 'vitest'
import type { ArtifactView } from '../api'
import { Broker, type CapabilityHandler, MESSAGE_BYTES, callOf } from './broker'

function artifact(capabilities: Record<string, Record<string, unknown>> | null): ArtifactView {
  return {
    id: '0123456789ab',
    version: 1,
    title: 't',
    kind: 'html',
    media_type: 'text/html',
    blob: 'b',
    size: 1,
    inline: false,
    capabilities,
  }
}

const echo: CapabilityHandler = { methods: ['echo'], call: async (_method, args) => args, perMinute: 2 }

describe('the bridge broker', () => {
  it('serves only what the version declared and this view has', async () => {
    const broker = new Broker({ thread: 't', artifact: artifact({ db: {} }), emit: () => undefined }, { db: echo, user: echo })
    expect(await broker.answer({ id: 1, cap: 'silt', method: 'use', args: { name: 'db' } })).toEqual({
      ok: true,
      value: { available: true, methods: ['echo'] },
    })
    expect(await broker.answer({ id: 2, cap: 'silt', method: 'use', args: { name: 'user' } })).toEqual({
      ok: true,
      value: { available: false },
    })
    const refused = await broker.answer({ id: 3, cap: 'user', method: 'echo', args: 1 })
    expect(refused).toMatchObject({ ok: false, error: { code: 'not_granted' } })
    expect(await broker.answer({ id: 4, cap: 'db', method: 'drop', args: null })).toMatchObject({
      ok: false,
      error: { code: 'refused' },
    })
  })

  it('refuses a call too large, and past its rate', async () => {
    let now = 0
    const broker = new Broker({ thread: 't', artifact: artifact({ db: {} }), emit: () => undefined }, { db: echo }, () => now)
    const big = 'x'.repeat(MESSAGE_BYTES + 1)
    expect(await broker.answer({ id: 1, cap: 'db', method: 'echo', args: big })).toMatchObject({
      ok: false,
      error: { code: 'too_large' },
    })
    expect(await broker.answer({ id: 2, cap: 'db', method: 'echo', args: 1 })).toEqual({ ok: true, value: 1 })
    expect(await broker.answer({ id: 3, cap: 'db', method: 'echo', args: 2 })).toEqual({ ok: true, value: 2 })
    expect(await broker.answer({ id: 4, cap: 'db', method: 'echo', args: 3 })).toMatchObject({
      ok: false,
      error: { code: 'rate_limited' },
    })
    now = 61_000
    expect(await broker.answer({ id: 5, cap: 'db', method: 'echo', args: 4 })).toEqual({ ok: true, value: 4 })
  })

  it('lets a capability take and give more than a message where it says so', async () => {
    const big = 'x'.repeat(MESSAGE_BYTES + 1)
    const roomy: CapabilityHandler = {
      methods: ['echo'],
      call: async (_method, args) => args,
      callBytes: 4 * MESSAGE_BYTES,
      answerBytes: 4 * MESSAGE_BYTES,
    }
    const broker = new Broker(
      { thread: 't', artifact: artifact({ db: {}, files: {} }), emit: () => undefined },
      { db: roomy, files: echo },
    )
    expect(await broker.answer({ id: 1, cap: 'db', method: 'echo', args: big })).toEqual({ ok: true, value: big })
    const files = await broker.answer({ id: 2, cap: 'files', method: 'echo', args: big })
    expect(files).toMatchObject({ ok: false, error: { code: 'too_large' } })
  })

  it('reads only well-formed calls', () => {
    expect(callOf({ silt: 'call', id: 1, cap: 'db', method: 'get', args: { path: 'a/b' } })).toEqual({
      id: 1,
      cap: 'db',
      method: 'get',
      args: { path: 'a/b' },
    })
    expect(callOf({ silt: 'call', id: '1', cap: 'db', method: 'get' })).toBeNull()
    expect(callOf({ silt: 'selection' })).toBeNull()
    expect(callOf(null)).toBeNull()
  })
})
