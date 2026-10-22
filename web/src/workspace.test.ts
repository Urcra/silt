import { expect, test } from 'vitest'
import type { Project, ThreadSummary } from './api'
import { inWorkspace, orgsOf, workspaceOf, workspaceValue } from './workspace'

const WORK = 'https://work.example'
const OSS = 'https://oss.example'

function thread(id: string, server: string | null, repo: string, local_only = false): ThreadSummary {
  return {
    id,
    repo,
    title: id,
    agent: null,
    created_ms: 0,
    messages: 0,
    edits: 0,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    writable: true,
    agent_state: null,
    last_ms: 0,
    merged: false,
    review: false,
    mine: { pinned: false, snoozed_until: null, snoozed_at: null, wake_on_activity: false, settled: false, seen_ms: null },
    visibility: null,
    server,
    local_only,
    cloud: null,
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
    threads: 0,
    open_mrs: 0,
    clone: null,
    home: null,
    server,
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
  ]) {
    expect(workspaceOf(workspaceValue(workspace))).toEqual(workspace)
  }
})
