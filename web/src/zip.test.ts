import { describe, expect, it } from 'vitest'
import { crc32, zipOf } from './zip'

describe('a zip', () => {
  it('checksums as every zip reader does', () => {
    expect(crc32(new TextEncoder().encode('123456789'))).toBe(0xcbf43926)
  })

  it('lists each file once, stored as it is, with the directory at the end', () => {
    const files = [
      { path: 'index.html', bytes: new TextEncoder().encode('<h1>Hi</h1>') },
      { path: 'data/n.json', bytes: new TextEncoder().encode('{"n":3}') },
    ]
    const zip = zipOf(files)
    const view = new DataView(zip.buffer)
    expect(view.getUint32(0, true)).toBe(0x04034b50)
    const end = zip.length - 22
    expect(view.getUint32(end, true)).toBe(0x06054b50)
    expect(view.getUint16(end + 10, true)).toBe(2)
    const text = new TextDecoder().decode(zip)
    expect(text).toContain('<h1>Hi</h1>')
    expect(text).toContain('data/n.json')
  })
})
