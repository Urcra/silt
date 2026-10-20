import { api, short } from '../api'
import { href } from '../router'
import { useLoad } from '../useLoad'
import { Search } from './Search'

export function ThreadList() {
  const threads = useLoad(api.threads, 'threads')
  return (
    <div>
      <Search />
      <h2 className="mb-2 text-lg font-semibold">Threads</h2>
      {threads.error && <p className="text-rose-700">{threads.error}</p>}
      <table className="w-full text-left text-sm">
        <thead className="text-xs text-neutral-500">
          <tr>
            <th className="py-1">Thread</th>
            <th>Title</th>
            <th>Where</th>
            <th>Messages</th>
            <th>Edits</th>
            <th>Repository</th>
          </tr>
        </thead>
        <tbody data-testid="threads">
          {[...(threads.data ?? [])].reverse().map((thread) => (
            <tr key={thread.id} className="border-t border-neutral-100">
              <td className="py-1 font-mono">
                <a className="text-sky-700" href={href({ page: 'thread', id: thread.id })}>
                  {short(thread.id)}
                </a>
              </td>
              <td>
                {thread.title ?? <span className="text-neutral-400">untitled</span>}
                {thread.forked_from && (
                  <span className="ml-2 text-xs text-violet-700">fork of {short(thread.forked_from)}</span>
                )}
              </td>
              <td>{thread.location}</td>
              <td>{thread.messages}</td>
              <td>{thread.edits}</td>
              <td className="text-xs text-neutral-500">{thread.repo}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
