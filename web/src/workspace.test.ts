import { expect, test } from 'vitest'
import type { Project, ThreadSummary } from './api'
import { inWorkspace, orgsOf, workspaceOf, workspaceValue } from './workspace'

const WORK = 'https://work.example'
const OSS = 'https://oss.example'

function thread(
  id: string,
  server: string | null,
  repo: string,
  local_only = false,
  workspace: string | null = null,
): ThreadSummary {
  return {
    id,
    repo,
    title: id,
    agent: null,
    created_ms: 0,
    messages: 0,
    edits: 0,
    shared: false,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    writable: true,
    agent_state: null,
    background: 0,
    last_ms: 0,
    merged: false,
    review: false,
    mine: {
      pinned: false,
      snoozed_until: null,
      snoozed_at: null,
      wake_on_activity: false,
      settled: false,
      seen_ms: null,
      reviewed_turn: null,
    },
    visibility: null,
    server,
    local_only,
    workspace,
    cloud: null,
    machine: null,
    agent_settings: null,
  }
}

function project(server: string, repo: string, org: string | null): Project {
  return {
    repo,
    name: repo,
    org,
    origin_url: null,
    visibility: null,
    default_agent: null,
    default_preset: null,
    git_notes: 'off',
    forge_comments: false,
    threads: 0,
    open_mrs: 0,
    clone: null,
    home: null,
    server,
    workspace: null,
    workspace_why: null,
  }
}

test('a workspace holds the threads of its server and org, or of this machine', () => {
  const orgs = orgsOf([project(WORK, 'acme/app', 'acme'), project(WORK, 'acme/tools', 'tools'), project(OSS, 'acme/app', null)])
  const app = thread('a', WORK, 'acme/app')
  const tools = thread('b', WORK, 'acme/tools')
  const open = thread('c', OSS, 'acme/app')
  const kept = thread('d', null, 'acme/app', true)
  const shown = (workspace: Parameters<typeof inWorkspace>[1]) =>
    [app, tools, open, kept].filter((each) => inWorkspace(each, workspace, orgs)).map((each) => each.id)
  expect(shown({ kind: 'all' })).toEqual(['a', 'b', 'c', 'd'])
  expect(shown({ kind: 'server', server: WORK, org: null })).toEqual(['a', 'b'])
  expect(shown({ kind: 'server', server: WORK, org: 'acme' })).toEqual(['a'])
  expect(shown({ kind: 'server', server: OSS, org: null })).toEqual(['c'])
  expect(shown({ kind: 'local' })).toEqual(['d'])
})

test('a workspace reads back from its select value', () => {
  for (const workspace of [
    { kind: 'all' } as const,
    { kind: 'local' } as const,
    { kind: 'server', server: WORK, org: null } as const,
    { kind: 'server', server: WORK, org: 'acme' } as const,
    { kind: 'workspace', id: 'work', server: WORK } as const,
    { kind: 'workspace', id: 'experiments', server: null } as const,
  ]) {
    expect(workspaceOf(workspaceValue(workspace))).toEqual(workspace)
  }
})

test("one of this machine's workspaces holds the threads made in it (ADR 0065)", () => {
  const work = thread('w', WORK, 'acme/app', false, 'work')
  const mine = thread('p', OSS, 'me/silt', false, 'personal')
  const shown = [work, mine]
    .filter((each) => inWorkspace(each, { kind: 'workspace', id: 'work', server: WORK }, new Map()))
    .map((each) => each.id)
  expect(shown).toEqual(['w'])
})
