// The routes in the first screen's chunk, the thread list and a thread;
// router.tsx loads every other page in its own chunk when it's opened.

import { useParams } from '@tanstack/react-router'
import { ThreadList } from './ThreadList'
import { ThreadView } from './ThreadView'

export const HomeRoute = ThreadList

export function ThreadRoute() {
  const { id } = useParams({ from: '/t/$id' })
  return <ThreadView key={id} id={id} />
}
