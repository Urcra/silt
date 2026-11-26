import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { checkedBySilt, nextPermission, ownModes, presetForOwnMode, startSettings, startingValue } from '../agentState'
import { type AgentSetting, api, type Preset } from '../api'
import { agentRefused, modeRefused, presetRefused, within } from '../rules'
import { useWorkspace } from '../workspace'
import { ATTACH, useAttachments } from '../attachments'
import { NEW_CHAT_DRAFT, useDraft } from '../drafts'
import { useHeldSend } from '../heldSend'
import { useMedia } from '../media'
import { keys, message } from '../query'
import { addSending } from '../sending'
import { hosted } from '../served'
import { Skeleton, toast } from '../ui'
import { AgentSettings } from './AgentSettings'
import { AttachedFiles } from './Attachments'
import { AgentChip, PermissionChip } from './AgentPickers'
import { UsageRing } from './UsageRing'
import { ComposerBox } from './Composer'
import { WorkspaceLine } from './NewThread'
import { Page } from './Page'

/**
 * A new chat (ADR 0059): New thread's composer with only what a chat needs. It works in a folder of its own, so there's
 * no project, no Local, Worktree or Cloud and no base branch; it's private, and starts with full access.
 */
export function NewChat() {
  const queries = useQueryClient()
  const navigate = useNavigate()
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const chatSettings = useQuery({ queryKey: keys.chatSettings, queryFn: api.chatSettings })
  const [chosenAgent, setAgent] = useState<string>()
  const [chosenSettings, setChosenSettings] = useState<Record<string, string>>({})
  const [chosenPreset, setChosenPreset] = useState<Preset>()
  // One of the agent's own modes to start it in, such as Copilot's Autopilot, from the permissions menu (ADR 0118).
  const [chosenMode, setChosenMode] = useState<string>()
  const setPreset = (preset: Preset) => {
    setChosenPreset(preset)
    setChosenMode(undefined)
  }
  const chooseMode = (mode: string) => {
    setChosenPreset(presetForOwnMode(mode))
    setChosenMode(mode)
  }
  const [prompt, setPrompt] = useDraft(NEW_CHAT_DRAFT)
  const [error, setError] = useState<string>()
  const [starting, setStarting] = useState(false)
  const agents = choices.data?.agents ?? []
  // In the workspace the sidebar shows, else the default, under its rules (ADR 0065).
  const shownWorkspace = useWorkspace()
  const workspacesQuery = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const workspaceId = shownWorkspace.kind === 'workspace' ? shownWorkspace.id : workspacesQuery.data?.default
  const workspace = workspacesQuery.data?.workspaces?.find((view) => view.workspace.id === workspaceId)
  const agent =
    (chosenAgent && !agentRefused(workspace, chosenAgent) ? chosenAgent : undefined) ??
    agents.find((candidate) => !agentRefused(workspace, candidate.id))?.id ??
    ''
  const seen = agents.find((candidate) => candidate.id === agent)
  const agentName = seen?.name ?? ''
  const wanted: Preset = chosenPreset ?? workspace?.workspace.defaults.chats_preset ?? chatSettings.data?.preset ?? 'full_access'
  const capped: Preset = within(wanted, workspace?.effective.max_preset) ? wanted : workspace!.effective.max_preset!
  const modes = seen?.modes?.available
  // The preset is the one control for every agent; the agent starts in the mode that does what it says (ADR 0114).
  const refusedPreset = (id: Preset) => presetRefused(workspace, id)
  const preset: Preset = refusedPreset(capped) && capped === 'auto' ? 'ask_before_commands' : capped
  const own = ownModes(seen?.modes)
  const inOwn = own.find((mode) => mode.id === chosenMode)?.id
  const refusedOwn = (mode: string) => modeRefused(workspace, agent, mode)
  const settings = startSettings(seen?.settings ?? [], modes)
  const settingValue = (setting: AgentSetting) =>
    startingValue(setting, chosenSettings[setting.id], workspace?.workspace.defaults.agents?.[agent]?.settings?.[setting.id])
  const { attached, add, remove, clear, busy, failure } = useAttachments()
  // A start asked for while files upload, which goes once they're there (ADR 0121).
  const startOnceUploaded = useRef<boolean | null>(null)
  const attach = (files: File[], pasted = false) => {
    setError(undefined)
    add(files, pasted).catch((failure: unknown) => setError(message(failure)))
  }
  const narrow = useMedia('(max-width: 767px)')
  const device = status.data?.servers?.[0]?.name ?? 'this machine'
  const synced = (status.data?.servers?.length ?? 0) > 0

  const start = (background: boolean) => {
    setError(undefined)
    if (failure) {
      setError(failure)
      return
    }
    if (busy) {
      startOnceUploaded.current = background
      return
    }
    if (!agent) return
    setStarting(true)
    api
      .startAgent(
        agent,
        {},
        {
          chat: true,
          workspace: workspaceId ?? null,
          mode: inOwn ?? null,
          settings: settings.map((setting) => [setting.id, settingValue(setting)]),
          preset,
          prompt: prompt.trim() ? prompt : null,
          attachments: attached,
        },
      )
      .then(
        (thread) => {
          if (!background && prompt.trim())
            addSending(
              thread,
              prompt,
              attached.map((file) => file.name),
              0,
            )
          clear()
          // The draft was this one's; New chat opens empty next time.
          setPrompt('')
          void queries.invalidateQueries({ queryKey: keys.threads })
          if (!background) {
            void navigate({ to: '/t/$id', params: { id: thread } })
            return
          }
          toast(`${agentName} started in a new chat`, 'ok')
        },
        (failure: unknown) => setError(message(failure)),
      )
      .finally(() => setStarting(false))
  }
  useEffect(() => {
    if (busy || startOnceUploaded.current === null) return
    const background = startOnceUploaded.current
    startOnceUploaded.current = null
    start(background)
  })

  const ready = !starting && Boolean(agent)
  const send = useHeldSend(choices.isPending, ready, () => start(false))
  return (
    <Page title="New chat" flush label="new chat">
      <div className="mx-auto flex w-full max-w-[760px] flex-1 flex-col justify-center gap-[18px] px-4 pt-10 pb-16 md:px-6 md:pt-14">
        <h2 className="text-center text-display font-medium tracking-[-0.015em] max-md:text-xl">What do you want to know?</h2>
        <form
          aria-label="new chat"
          className="flex flex-col"
          onSubmit={(event) => {
            event.preventDefault()
            send()
          }}
        >
          <ComposerBox
            id="chat-prompt"
            label="first prompt"
            rows={4}
            value={prompt}
            onChange={setPrompt}
            onSend={send}
            onCycle={() => {
              const next = nextPermission(preset, inOwn, own, refusedPreset, refusedOwn)
              if (next?.mode !== undefined) chooseMode(next.mode)
              else if (next) setPreset(next.preset)
            }}
            placeholder="Ask anything: which part to buy, how something works, a quick script. It works in a folder of its own."
            onFiles={attach}
            commands={seen?.commands}
            below={<AttachedFiles files={attached} onRemove={remove} />}
            tools={
              <>
                {agents.length > 0 ? (
                  <AgentChip
                    value={agent}
                    agents={agents}
                    onChange={(chosen) => {
                      setAgent(chosen)
                      setChosenSettings({})
                      if (chosenMode !== undefined) {
                        setChosenMode(undefined)
                        setChosenPreset(undefined)
                      }
                    }}
                    refused={(id) => agentRefused(workspace, id)}
                  />
                ) : choices.isPending ? (
                  <Skeleton className="h-6 w-20" />
                ) : (
                  <span className="px-2 text-xs text-fg-2">No agent found</span>
                )}
                <PermissionChip
                  checked={checkedBySilt(modes)}
                  value={preset}
                  onChange={setPreset}
                  refused={refusedPreset}
                  own={
                    own.length > 0
                      ? { agent, name: agentName, modes: own, mode: inOwn, onMode: chooseMode, refused: refusedOwn }
                      : undefined
                  }
                />
                <AgentSettings
                  settings={settings}
                  value={settingValue}
                  onSetting={(id, value) => setChosenSettings({ ...chosenSettings, [id]: value })}
                  narrow={narrow}
                />
                {agent && <UsageRing agent={agent} />}
              </>
            }
            voice={!hosted()}
            attach={ATTACH}
            enter="start"
            send={{ label: 'Start', title: 'Start and open it', disabled: !ready, busy: starting, submit: true }}
            menu={[{ label: 'Start in the background', icon: 'clock' as const, onSelect: () => start(true), disabled: !ready }]}
          />
          <p className="mt-2.5 px-0.5 text-xs text-fg-2">
            {`On ${device}, in a folder of its own. Private to you${synced ? ', and on your other devices too' : ''}.`}
          </p>
          {workspace && workspacesQuery.data?.saved && <WorkspaceLine view={workspace} />}
          {choices.isSuccess && agents.length === 0 && (
            <p className="mt-1 px-0.5 text-xs text-warn-fg">
              No agent CLI is on the daemon's <code className="font-mono">PATH</code>. Install one, such as Claude Code, then run{' '}
              <code className="font-mono">silt daemon stop</code> so the next start finds it.
            </p>
          )}
          {error && <p className="mt-2 text-sm whitespace-pre-wrap text-danger">{error}</p>}
        </form>
      </div>
    </Page>
  )
}
