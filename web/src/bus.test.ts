import { QueryClient, QueryObserver } from '@tanstack/react-query'
import { expect, test, vi } from 'vitest'
import { Refetcher } from './bus'

test('a change announced while its fetch is under way is fetched once that fetch ends', async () => {
  const queries = new QueryClient()
  let stored = 'before'
  const pending: (() => void)[] = []
  const queryFn = vi.fn(() => {
    const read = stored
    return new Promise<string>((resolve) => pending.push(() => resolve(read)))
  })
  const observer = new QueryObserver(queries, { queryKey: ['thread', 't', 'history'], queryFn })
  const unsubscribe = observer.subscribe(() => {})
  const refetcher = new Refetcher(queries)
  const stop = refetcher.start()
  const answer = async () => {
    await vi.waitFor(() => expect(pending.length).toBeGreaterThan(0))
    pending.shift()!()
  }
  await answer()
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('before'))

  refetcher.refetch(['thread', 't'])
  await vi.waitFor(() => expect(queryFn).toHaveBeenCalledTimes(2))
  stored = 'after'
  refetcher.refetch(['thread', 't'])
  expect(queryFn).toHaveBeenCalledTimes(2)
  await answer()
  await answer()
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('after'))
  expect(queryFn).toHaveBeenCalledTimes(3)

  stop()
  unsubscribe()
})
