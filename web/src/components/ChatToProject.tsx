import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type ThreadSummary } from '../api'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { Button, Icon, Input, Select, buttonClass, cx, toast } from '../ui'

/**
 * What a chat can become (ADR 0059): a project of its own, its folder as it is, or a new thread in a project that starts from the
 * chat's conversation, with its files copied in.
 */
export function ChatToProject({ thread, summary, onDone }: { thread: string; summary: ThreadSummary; onDone: () => void }) {
  const navigate = useNavigate()
  const queries = useQueryClient()
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const clones = choices.data?.clones ?? []
  const agents = choices.data?.agents ?? []
  const [making, setMaking] = useState(false)
  const [continuing, setContinuing] = useState(false)
  const [clone, setClone] = useState('')
  const [copy, setCopy] = useState(true)
  const [copyTo, setCopyTo] = useState(`notes/chat-${short(thread)}`)
  const [prompt, setPrompt] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const chosen = clone || clones[0]?.path || ''
  const agent =
    agents.find((candidate) => candidate.name === summary.agent || candidate.id === summary.agent)?.id ?? agents[0]?.id ?? ''
  const failed = (failure: unknown) => setError(message(failure))
  const makeProject = () => {
    setBusy(true)
    api
      .makeProject(thread)
      .then((project) => {
        toast(`The chat is now the project ${projectName(project)}`, 'ok')
        void queries.invalidateQueries({ queryKey: keys.threads })
        void queries.invalidateQueries({ queryKey: keys.projects })
        void queries.invalidateQueries({ queryKey: keys.agents })
        onDone()
      }, failed)
      .finally(() => setBusy(false))
  }
  const continueIn = () => {
    setBusy(true)
    api
      .startAgent(
        agent,
        { clone: chosen },
        {
          here: true,
          continues: thread,
          copy_to: copy ? copyTo.trim() : null,
          prompt: prompt.trim() || 'Carry on from the chat.',
        },
      )
      .then((started) => {
        void queries.invalidateQueries({ queryKey: keys.threads })
        onDone()
        void navigate({ to: '/t/$id', params: { id: started } })
      }, failed)
      .finally(() => setBusy(false))
  }
  return (
    <div className="flex flex-col gap-1.5">
      {!making ? (
        <button
          type="button"
          className={cx(buttonClass('ghost'), 'justify-start font-normal text-fg-1')}
          onClick={() => setMaking(true)}
        >
          <Icon name="folder" />
          Make it a project…
        </button>
      ) : (
        <div className="flex flex-col gap-1.5 rounded-sm border border-line p-2 text-xs">
          <p className="text-fg-2">
            Its folder becomes a project of its own, with this chat and its files in it. Add a remote later to share it.
          </p>
          <span className="flex justify-end gap-1.5">
            <Button variant="ghost" onClick={() => setMaking(false)}>
              Cancel
            </Button>
            <Button variant="primary" disabled={busy} onClick={makeProject}>
              Make it a project
            </Button>
          </span>
        </div>
      )}
      {!continuing ? (
        <button
          type="button"
          className={cx(buttonClass('ghost'), 'justify-start font-normal text-fg-1')}
          onClick={() => setContinuing(true)}
        >
          <Icon name="branch" />
          Continue in a project…
        </button>
      ) : (
        <form
          className="flex flex-col gap-1.5 rounded-sm border border-line p-2 text-xs"
          onSubmit={(event) => {
            event.preventDefault()
            continueIn()
          }}
        >
          {clones.length === 0 ? (
            <p className="text-fg-2">No project on this machine yet.</p>
          ) : (
            <Select aria-label="project to continue in" value={chosen} onChange={(event) => setClone(event.target.value)}>
              {clones.map((candidate) => (
                <option key={candidate.path} value={candidate.path}>
                  {candidate.repo ? projectName(candidate.repo) : candidate.path}
                </option>
              ))}
            </Select>
          )}
          <label className="flex items-center gap-1.5 text-fg-2">
            <input type="checkbox" checked={copy} onChange={(event) => setCopy(event.target.checked)} />
            Copy the chat&apos;s files to
          </label>
          {copy && (
            <Input
              aria-label="folder in the project"
              value={copyTo}
              onChange={(event) => setCopyTo(event.target.value)}
              className="font-mono text-xs"
            />
          )}
          <Input
            aria-label="first prompt there"
            placeholder="Carry on from the chat."
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
          />
          <p className="text-fg-2">A new thread in your clone, its agent starting from this chat&apos;s conversation.</p>
          <span className="flex justify-end gap-1.5">
            <Button variant="ghost" onClick={() => setContinuing(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !chosen || !agent}>
              Start there
            </Button>
          </span>
        </form>
      )}
      {error && <p className="px-1 text-xs text-danger">{error}</p>}
    </div>
  )
}
