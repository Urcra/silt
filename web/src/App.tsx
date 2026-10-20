import { MergeRequest, MergeRequests } from './components/MergeRequests'
import { ThreadList } from './components/ThreadList'
import { ThreadView } from './components/ThreadView'
import { href, useRoute } from './router'

function App() {
  const route = useRoute()
  return (
    <main className="mx-auto max-w-6xl p-6">
      <header className="mb-6 flex items-baseline gap-6 border-b border-neutral-200 pb-3">
        <h1 className="text-2xl font-semibold">Silt</h1>
        <nav className="flex gap-4 text-sm">
          <a href={href({ page: 'threads' })} className="text-sky-700">
            Threads
          </a>
          <a href={href({ page: 'mrs' })} className="text-sky-700">
            Merge requests
          </a>
        </nav>
      </header>
      {route.page === 'threads' && <ThreadList />}
      {route.page === 'thread' && <ThreadView id={route.id} />}
      {route.page === 'mrs' && <MergeRequests />}
      {route.page === 'mr' && <MergeRequest id={route.id} />}
    </main>
  )
}

export default App
