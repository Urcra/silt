// The routes in the first screen's chunk, the new thread, All threads and a thread;
// router.tsx loads every other page in its own chunk when it's opened.

import { useParams, useSearch } from '@tanstack/react-router'
import { hosted } from '../served'
import { NewThread } from './NewThread'
import { ThreadList } from './ThreadList'
import { ThreadView } from './ThreadView'

/** `/` is an empty new thread (ADR 0044); a server starts no agents, so there it lists the threads. */
export function HomeRoute() {
  const search = useSearch({ from: '/' })
  // A link to another project or to the picker starts the page afresh.
  return hosted() ? <ThreadList /> : <NewThread key={`${search.project ?? ''}|${search.pick ?? ''}|${search.prompt ?? ''}`} />
}

/** All threads, in the first chunk too: a server's `/` is it. */
export const ThreadsRoute = ThreadList

export function ThreadRoute() {
  const { id } = useParams({ from: '/t/$id' })
  return <ThreadView key={id} id={id} />
}
