import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { PRESETS, modeForPreset, permissionModes, startSettings, startingValue } from '../agentState'
import { type AgentSetting, api, type Preset } from '../api'
import { agentRefused, presetRefused, within } from '../rules'
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
import { ChipSelect } from './ChipSelect'
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
  const [chosenPreset, setPreset] = useState<Preset>()
  const [mode, setMode] = useState<string>()
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
  const preset: Preset = within(wanted, workspace?.effective.max_preset) ? wanted : workspace!.effective.max_preset!
  const modes = seen?.modes?.available
  // Claude's modes are its permissions (ADR 0048): the chat's preset picks one until the person does.
  const modesArePermissions = permissionModes(modes)
  const chosenMode = mode ?? (modesArePermissions ? modeForPreset(preset, modes) : undefined) ?? seen?.modes?.current
  const settings = startSettings(seen?.settings ?? [], modes)
  const settingValue = (setting: AgentSetting) =>
    startingValue(setting, chosenSettings[setting.id], workspace?.workspace.defaults.agents?.[agent]?.settings?.[setting.id])
  const { attached, add, remove, clear } = useAttachments()
  const attach = (files: File[], pasted = false) => {
    setError(undefined)
    add(files, pasted).catch((failure: unknown) => setError(message(failure)))
  }
  const narrow = useMedia('(max-width: 767px)')
  const device = status.data?.servers?.[0]?.name ?? 'this machine'
  const synced = (status.data?.servers?.length ?? 0) > 0

  const start = (background: boolean) => {
    setError(undefined)
    if (!agent) return
    setStarting(true)
    api
      .startAgent(
        agent,
        {},
        {
          chat: true,
          workspace: workspaceId ?? null,
          mode: modesArePermissions ? (mode ?? null) : null,
          settings: settings.map((setting) => [setting.id, settingValue(setting)]),
          preset,
          prompt: prompt.trim() ? prompt : null,
          attachments: attached,
        },
      )
      .then(
        (thread) => {
          if (!background && prompt.trim()) addSending(thread, prompt, attached.map((file) => file.name), 0)
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

  const ready = !starting && Boolean(agent)
  const send = useHeldSend(choices.isPending, ready, () => start(false))
  return (
    <Page title="New chat" flush label="new chat">
      <div className="mx-auto flex w-full max-w-[760px] flex-1 flex-col justify-center gap-[18px] px-4 pt-10 pb-16 md:px-6 md:pt-14">
        <h2 className="text-center text-[26px] font-medium tracking-[-0.015em] max-md:text-xl">What do you want to know?</h2>
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
              if (modesArePermissions && modes) setMode(modes[(modes.findIndex((choice) => choice.id === chosenMode) + 1) % modes.length]!.id)
              else setPreset(PRESETS[(PRESETS.findIndex((choice) => choice.id === preset) + 1) % PRESETS.length]!.id)
            }}
            placeholder="Ask anything: which part to buy, how something works, a quick script. It works in a folder of its own."
            onFiles={attach}
            commands={seen?.commands}
            below={<AttachedFiles files={attached} onRemove={remove} />}
            tools={
              <>
                {agents.length > 0 ? (
                  <ChipSelect
                    ghost
                    label="agent to start"
                    value={agent}
                    onChange={(chosen) => {
                      setAgent(chosen)
                      setMode(undefined)
                      setChosenSettings({})
                    }}
                    options={agents.map((choice) => ({ id: choice.id, name: choice.name, description: choice.description, refused: agentRefused(workspace, choice.id) }))}
                  />
                ) : choices.isPending ? (
                  <Skeleton className="h-6 w-20" />
                ) : (
                  <span className="px-2 text-xs text-fg-2">No agent found</span>
                )}
                <AgentSettings
                  modes={modesArePermissions ? modes : undefined}
                  mode={chosenMode}
                  onMode={setMode}
                  settings={settings}
                  value={settingValue}
                  onSetting={(id, value) => setChosenSettings({ ...chosenSettings, [id]: value })}
                  narrow={narrow}
                />
                {!modesArePermissions && (
                  <ChipSelect
                    ghost
                    label="permissions"
                    icon="lock"
                    value={preset}
                    onChange={(chosen) => setPreset(chosen as Preset)}
                    options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label, description: choice.says, refused: presetRefused(workspace, choice.id) }))}
                  />
                )}
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
            <p className="mt-1 px-0.5 text-xs text-warn">
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
