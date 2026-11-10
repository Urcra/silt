import { describe, expect, it } from 'vitest'
import { Downsampler, Framer, insertWords, microphoneProblem } from './voice'

const audio = (...samples: number[]) => Float32Array.from(samples)

describe('Downsampler', () => {
  it('averages what each output sample covers', () => {
    expect([...new Downsampler(48_000).push(audio(0, 0.3, 0.6, 1, 1, 1))]).toEqual([expect.closeTo(0.3), expect.closeTo(1)])
  })

  it('carries a part-filled sample over to the next push', () => {
    const downsampler = new Downsampler(48_000)
    expect([...downsampler.push(audio(1, 1))]).toEqual([])
    expect([...downsampler.push(audio(1))]).toEqual([1])
  })

  it('keeps the right pace at a rate that does not divide evenly', () => {
    expect(new Downsampler(44_100).push(new Float32Array(44_100)).length).toBeGreaterThanOrEqual(15_999)
    expect(new Downsampler(44_100).push(new Float32Array(44_100)).length).toBeLessThanOrEqual(16_000)
  })

  it('leaves audio already at the speech rate alone', () => {
    expect([...new Downsampler(16_000).push(audio(0.5, -0.5))]).toEqual([0.5, -0.5])
  })

  it('refuses to raise the rate', () => {
    expect(() => new Downsampler(8_000)).toThrow()
  })
})

describe('Framer', () => {
  it('holds samples until a frame is whole', () => {
    const framer = new Framer(3)
    expect(framer.push(audio(1, 2))).toEqual([])
    expect(framer.push(audio(3, 4, 5, 6, 7)).map((frame) => [...frame])).toEqual([
      [1, 2, 3],
      [4, 5, 6],
    ])
  })

  it('gives up what is left over once', () => {
    const framer = new Framer(3)
    framer.push(audio(1))
    expect([...framer.rest()!]).toEqual([1])
    expect(framer.rest()).toBeUndefined()
  })
})

describe('insertWords', () => {
  it('fills an empty box', () => {
    expect(insertWords('', 0, 0, 'Hello.')).toEqual({ value: 'Hello.', caret: 6 })
  })

  it('sets words apart from the text around them', () => {
    expect(insertWords('ab cd', 2, 2, 'x').value).toBe('ab x cd')
    expect(insertWords('abcd', 2, 2, 'x').value).toBe('ab x cd')
  })

  it('adds no second space', () => {
    expect(insertWords('ab ', 3, 3, 'x').value).toBe('ab x')
  })

  it('keeps punctuation against the words', () => {
    expect(insertWords('ab.', 2, 2, 'x').value).toBe('ab x.')
  })

  it('replaces the selection and leaves the caret after the words', () => {
    expect(insertWords('one two three', 4, 7, 'six')).toEqual({ value: 'one six three', caret: 7 })
  })
})

describe('microphoneProblem', () => {
  const named = (name: string) => Object.assign(new Error('x'), { name })

  it('says what went wrong', () => {
    expect(microphoneProblem(named('NotAllowedError'))).toMatch(/refused/)
    expect(microphoneProblem(named('NotFoundError'))).toMatch(/No microphone/)
    expect(microphoneProblem(named('InvalidStateError'))).toMatch(/gst-plugins-good/)
    expect(microphoneProblem('odd')).toMatch(/could not be opened/)
  })
})
