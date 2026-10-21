import { Link, Outlet, useParams } from '@tanstack/react-router'
import { MergeRequest } from './MergeRequests'
import { ThreadView } from './ThreadView'

export function Layout() {
  return (
    <main className="mx-auto max-w-6xl p-6">
      <header className="mb-6 flex items-baseline gap-6 border-b border-neutral-200 pb-3">
        <h1 className="text-2xl font-semibold">Silt</h1>
        <nav className="flex gap-4 text-sm">
          <Link to="/" className="text-sky-700">
            Threads
          </Link>
          <Link to="/mrs" className="text-sky-700">
            Merge requests
          </Link>
          <Link to="/inbox" className="text-sky-700">
            Inbox
          </Link>
        </nav>
      </header>
      <Outlet />
    </main>
  )
}

export function ThreadPage() {
  const { id } = useParams({ from: '/t/$id' })
  return <ThreadView id={id} />
}

export function MergeRequestPage() {
  const { id } = useParams({ from: '/mr/$id' })
  return <MergeRequest id={id} />
}
