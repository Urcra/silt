import { type KeyboardEvent, type RefObject, useLayoutEffect, useRef } from 'react'

interface Snapshot {
  value: string
  start: number
  end: number
  direction: 'forward' | 'backward' | 'none'
}

const snapshot = (value: string, box: HTMLTextAreaElement | null): Snapshot => ({
  value,
  start: box?.selectionStart ?? value.length,
  end: box?.selectionEnd ?? value.length,
  direction: box?.selectionDirection ?? 'none',
})

/** Controlled values can replace the browser's undo stack, so keep the draft's edits here. */
export function useComposerHistory(
  id: string,
  value: string,
  box: RefObject<HTMLTextAreaElement | null>,
  onChange: (value: string) => void,
) {
  const history = useRef({
    id,
    current: snapshot(value, null),
    past: [] as Snapshot[],
    future: [] as Snapshot[],
    kind: '',
    time: 0,
  })
  const pending = useRef<Snapshot | null>(null)

  useLayoutEffect(() => {
    const state = history.current
    // Sending, loading another draft or changing threads starts a fresh history.
    if (state.id !== id || state.current.value !== value) {
      history.current = { id, current: snapshot(value, box.current), past: [], future: [], kind: '', time: 0 }
      pending.current = null
    }
    if (pending.current) {
      const { start, end, direction } = pending.current
      box.current?.setSelectionRange(start, end, direction)
      pending.current = null
    }
  }, [id, value, box])

  const select = () => {
    const state = history.current
    const next = snapshot(state.current.value, box.current)
    if (next.start !== state.current.start || next.end !== state.current.end) state.kind = ''
    state.current = next
  }

  const edit = (text: string, kind = '') => {
    const state = history.current
    if (text === state.current.value) return
    const now = Date.now()
    const groupable = ['insertText', 'deleteContentBackward', 'deleteContentForward', 'insertCompositionText'].includes(kind)
    const collapsed = state.current.start === state.current.end
    if (!groupable || !collapsed || state.kind !== kind || now - state.time > 1000) {
      state.past.push(state.current)
      if (state.past.length > 100) state.past.shift()
    }
    state.future = []
    state.current = snapshot(text, box.current)
    state.kind = groupable && collapsed ? kind : ''
    state.time = now
    onChange(text)
  }

  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return false
    const key = event.key.toLowerCase()
    const modified = (event.ctrlKey || event.metaKey) && !event.altKey
    const undo = modified && key === 'z' && !event.shiftKey
    const redo =
      modified && ((key === 'z' && event.shiftKey) || (key === 'y' && event.ctrlKey && !event.metaKey && !event.shiftKey))
    if (!undo && !redo) {
      select()
      return false
    }
    event.preventDefault()
    const state = history.current
    const from = undo ? state.past : state.future
    const to = undo ? state.future : state.past
    const next = from.pop()
    if (next) {
      to.push(snapshot(state.current.value, box.current))
      state.current = next
      pending.current = next
      onChange(next.value)
    }
    state.kind = ''
    return true
  }

  return { edit, select, keyDown }
}
