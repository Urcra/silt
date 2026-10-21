import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { api, short } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { Icon, Tag } from '../ui'
import { Search } from './Search'

export function ThreadList() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  return (
    <div>
      <Search />
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-lg font-semibold">Threads</h2>
        <Link
          to="/new"
          className="inline-flex items-center gap-1.5 rounded-sm bg-accent px-2.5 py-1 text-xs font-medium text-on-accent hover:bg-accent-hover"
        >
          <Icon name="plus" />
          New thread
        </Link>
      </div>
      {threads.error && <p className="text-danger">{message(threads.error)}</p>}
      <table className="w-full text-left text-sm">
        <thead className="font-mono text-[11px] tracking-wider text-fg-3 uppercase">
          <tr>
            <th className="py-1">Thread</th>
            <th>Title</th>
            <th>Agent</th>
            <th>Where</th>
            <th>Messages</th>
            <th>Edits</th>
            <th>Repository</th>
          </tr>
        </thead>
        <tbody data-testid="threads">
          {[...(threads.data ?? [])].reverse().map((thread) => (
            <tr key={thread.id} className="border-t border-line-subtle hover:bg-surface-1">
              <td className="py-1 font-mono">
                <Link className="text-accent-fg" to="/t/$id" params={{ id: thread.id }}>
                  {short(thread.id)}
                </Link>
              </td>
              <td>
                {thread.title ?? <span className="text-fg-3">untitled</span>}
                {thread.forked_from && (
                  <Tag tone="info" className="ml-2">
                    fork of {short(thread.forked_from)}
                  </Tag>
                )}
              </td>
              <td>{thread.agent ?? ''}</td>
              <td>{thread.location}</td>
              <td>{thread.messages}</td>
              <td>{thread.edits}</td>
              <td className="font-mono text-xs text-fg-3">{thread.repo}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
