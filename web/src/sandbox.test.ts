import { expect, test } from 'vitest'
import type { Policy, WorkspaceView } from './api'
import { choiceOf, chosenPolicy, reachOf, sameChoice } from './sandbox'

function work(own: Policy, org: Policy | null): WorkspaceView {
  return {
    workspace: { id: 'work', name: 'Work', server: null, rules: [], defaults: {}, env: {}, policy: own },
    org_policy: org ? { org: 'acme', server: 'https://silt.acme.dev', policy: org } : null,
    waiting_for_policy: false,
    effective: { ...own, ...org },
    projects: [],
  }
}

test("a thread starts from its workspace's own choice", () => {
  expect(choiceOf({})).toEqual({ on: false, network: 'full', devices: 'none', gpu: false, docker: false, ssh_agent: false })
  const own = choiceOf({ silt_sandbox: true, network: 'gateway', gpu: true })
  expect(own).toMatchObject({ on: true, network: 'gateway', gpu: true })
  expect(sameChoice(own, { ...own })).toBe(true)
  expect(sameChoice(own, { ...own, gpu: false })).toBe(false)
})

test("a thread's choice replaces the workspace's, and its org still holds it", () => {
  const view = work({ ports: [5432] }, null)
  const wide = chosenPolicy(view, { ...choiceOf({}), on: true, network: 'host', docker: true })
  expect(wide.silt_sandbox).toBe(true)
  expect(reachOf(wide)).toMatchObject({ network: 'host', docker: true, ports: [5432] })

  const held = chosenPolicy(work({}, { silt_sandbox: true, network: 'gateway', refuse: ['docker'] }), {
    ...choiceOf({}),
    network: 'host',
    docker: true,
  })
  expect(held.silt_sandbox).toBe(true)
  expect(reachOf(held)).toMatchObject({ network: 'gateway', docker: false })
})
