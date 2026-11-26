import { useEffect, useState } from 'react'
import type { StateChange, ThreadSummary } from './api'

/** Where the desktop app can show Silt from: this computer, each WSL distribution (ADR 0045), and each machine over SSH (ADR 0108). */
export interface Backend {
  id: string
  label: string
  kind?: 'native' | 'wsl' | 'ssh'
  /** A machine's `user@host`, or a `Host` from `~/.ssh/config`. */
  destination?: string
  port?: number | null
  state?: 'idle' | 'connecting' | 'connected' | 'failed'
  /** Why it couldn't connect, when `state` is failed. */
  reason?: string | null
  /** What it's doing, when `state` is connecting. */
  step?: string | null
  /** When `state` last changed, in milliseconds since the epoch. */
  since?: number | null
  lastShown?: number | null
  /** The `silt --version` it answered with; This computer's is the build a machine is offered to update to (ADR 0109). */
  version?: string | null
  /** The window is switching to it, and still shows another. */
  switching?: boolean
  /** Silt is being installed or updated there. */
  installing?: boolean
  /** It failed only because `silt` isn't there, which installing fixes. */
  canInstall?: boolean
  /** An update waits for this many agents to finish before its daemon restarts on it. */
  pendingRestart?: number | null
  /** The colour its threads are marked with (ADR 0132). */
  colour?: MachineColour
  /** This computer's name, which its threads are marked with on other machines' pages. */
  host?: string | null
}

/** The colours a machine can have, in the order the desktop app gives them out. */
export const MACHINE_COLOURS = ['teal', 'plum', 'blue', 'rose', 'sand', 'ink'] as const
export type MachineColour = (typeof MACHINE_COLOURS)[number]

/** How a thread on another machine is told apart in the list (ADR 0132). */
export const MARKS = ['chip', 'colour-chip', 'line', 'stripe', 'initial', 'nothing'] as const
export type Mark = (typeof MARKS)[number]

/** How the person chose to list and mark threads, which every machine's page follows. */
export interface MachinesView {
  list: 'all' | 'one'
  mark: Mark
  markHere: boolean
}

/** One other machine's threads, as the desktop app last heard them. */
export interface Listing {
  id: string
  state: 'loading' | 'ready' | 'lost'
  threads: ThreadSummary[]
}

/** A machine as the add and edit form fills it in. */
export interface MachineFields {
  name: string
  destination: string
  port?: number | null
}

/** One step of testing a machine, as the desktop app reports it. */
export interface TestStep {
  state: 'waiting' | 'running' | 'done' | 'failed'
  text: string
}

export interface MachineTest extends MachineFields {
  /** Testing it, or installing Silt there. */
  action?: 'test' | 'install'
  steps: TestStep[]
  done: boolean
  ok: boolean
  /** It failed only because `silt` isn't there. */
  canInstall?: boolean
}

/** Why switching to a backend failed, which leaves the window where it was. */
export interface SwitchFailure {
  id: string
  label: string
  reason: string
}

declare global {
  interface Window {
    /** Set by the desktop app's window: where it can show Silt from, sent again as that changes. */
    __siltBackends?: Backend[]
    __siltBackend?: string
    /** Set by the desktop app's window: the commit it was built from (ADR 0100), the key its machines links carry, and how threads are listed (ADR 0132). */
    __siltDesktop?: { commit?: string; key?: string; view?: MachinesView }
    /** Set by the desktop app's window: the threads of every other connected machine (ADR 0132). */
    __siltThreads?: Listing[]
  }
}

function useEvent<T>(name: string, heard: (detail: T) => void) {
  useEffect(() => {
    const listener = (event: Event) => heard((event as CustomEvent<T>).detail)
    window.addEventListener(name, listener)
    return () => window.removeEventListener(name, listener)
  }, [name, heard])
}

/** The backends the desktop app offers and the one it shows, or none outside the app. */
export function useBackends(): { backends: Backend[]; current: string | undefined } {
  const [current, setCurrent] = useState(() => window.__siltBackend)
  const [backends, setBackends] = useState(() => window.__siltBackends ?? [])
  useEffect(() => {
    const shown = (event: Event) => setCurrent((event as CustomEvent<string>).detail)
    const listed = () => setBackends(window.__siltBackends ?? [])
    window.addEventListener('silt-backend', shown)
    window.addEventListener('silt-backends', listed)
    return () => {
      window.removeEventListener('silt-backend', shown)
      window.removeEventListener('silt-backends', listed)
    }
  }, [])
  return { backends, current }
}

const DEFAULT_VIEW: MachinesView = { list: 'all', mark: 'chip', markHere: false }

/** How threads are listed and marked: the desktop app's choice, or one machine outside it. */
export function useMachinesView(): MachinesView {
  const read = () =>
    inDesktop() ? { ...DEFAULT_VIEW, ...window.__siltDesktop?.view } : { ...DEFAULT_VIEW, list: 'one' as const }
  const [view, setView] = useState(read)
  useEffect(() => {
    const changed = () => setView(read())
    window.addEventListener('silt-view', changed)
    return () => window.removeEventListener('silt-view', changed)
  }, [])
  return view
}

/** Every other connected machine's threads, as the desktop app last heard them; none outside it. */
export function useListings(): Listing[] {
  const [listings, setListings] = useState(() => window.__siltThreads ?? [])
  useEffect(() => {
    const heard = () => setListings(window.__siltThreads ?? [])
    window.addEventListener('silt-threads', heard)
    return () => window.removeEventListener('silt-threads', heard)
  }, [])
  return listings
}

/** The link that asks the desktop app to change how threads are listed or marked; it carries the window's key. */
export function viewLink(path: '' | 'mark' | 'colour', fields: Record<string, string>, key = window.__siltDesktop?.key): string {
  return `silt://view/${path}?${new URLSearchParams({ ...fields, key: key ?? '' })}`
}

/** Lists every connected machine's threads, or the shown one's. */
export function setMachinesList(list: 'all' | 'one') {
  tellWindow(viewLink('', { list }))
}

export function setMark(mark: Mark, here: boolean) {
  tellWindow(viewLink('mark', { mark, here: here ? '1' : '0' }))
}

export function setMachineColour(backend: string, colour: MachineColour) {
  tellWindow(viewLink('colour', { backend, colour }))
}

/** The link that pins, snoozes or settles a thread on another machine, through the desktop app. */
export function stateLink(backend: string, thread: string, change: StateChange, key = window.__siltDesktop?.key): string {
  return `silt://threads/state?${new URLSearchParams({ backend, thread, change: JSON.stringify(change), key: key ?? '' })}`
}

export function changeElsewhere(backend: string, thread: string, change: StateChange) {
  tellWindow(stateLink(backend, thread, change))
}

export function openLink(backend: string, thread: string): string {
  return `silt://open/?${new URLSearchParams({ backend, thread })}`
}

/** Switches the window to `backend` and opens the thread there. */
export function openElsewhere(backend: string, thread: string) {
  tellWindow(openLink(backend, thread))
}

export function newThreadLink(backend: string, draft: string, origin: string): string {
  const fields: Record<string, string> = { backend, new: '1' }
  if (origin) fields.origin = origin
  if (draft) fields.draft = draft
  return `silt://open/?${new URLSearchParams(fields)}`
}

/** Switches the window to `backend`'s New thread, with the draft and the project's origin. */
export function newThreadOn(backend: string, draft: string, origin: string) {
  tellWindow(newThreadLink(backend, draft, origin))
}

/** Calls `heard` when switching backends fails and the window stays where it was. */
export function useSwitchFailed(heard: (failure: SwitchFailure) => void) {
  useEvent('silt-backend-failed', heard)
}

/** The latest report of testing a machine, as its steps go by. */
export function useMachineTest(): [MachineTest | undefined, (test: MachineTest | undefined) => void] {
  const [test, setTest] = useState<MachineTest>()
  useEvent('silt-machine-test', setTest)
  return [test, setTest]
}

/** The link asking the desktop app to change its machines; it carries the window's key, so another page's can't (ADR 0108). */
export function machinesLink(
  action: 'add' | 'change' | 'remove' | 'test' | 'install' | 'disconnect',
  fields: Record<string, string | number | null | undefined>,
  key: string | undefined = window.__siltDesktop?.key,
): string {
  const query = new URLSearchParams()
  for (const [name, value] of Object.entries(fields)) {
    if (value !== undefined && value !== null && value !== '') query.set(name, String(value))
  }
  query.set('key', key ?? '')
  return `silt://machines/${action}?${query}`
}

const asFields = (machine: MachineFields) => ({ name: machine.name, destination: machine.destination, port: machine.port })

export function addMachine(machine: MachineFields) {
  tellWindow(machinesLink('add', asFields(machine)))
}

export function changeMachine(was: string, machine: MachineFields) {
  tellWindow(machinesLink('change', { ...asFields(machine), was }))
}

export function removeMachine(name: string) {
  tellWindow(machinesLink('remove', { name }))
}

export function disconnectMachine(name: string) {
  tellWindow(machinesLink('disconnect', { name }))
}

/** Tests reaching `machine`, reported through `useMachineTest`; `add` adds it once it passes. */
export function testMachine(machine: MachineFields, add: boolean) {
  tellWindow(machinesLink('test', { ...asFields(machine), add: add ? 1 : undefined }))
}

/** Installs this computer's build of Silt on `machine`, or updates the one there to it, reported through `useMachineTest`; `add` adds it once it connects (ADR 0109). */
export function installSilt(machine: MachineFields, add: boolean) {
  tellWindow(machinesLink('install', { ...asFields(machine), add: add ? 1 : undefined }))
}

/** Asks the desktop app's window to show Silt from `backend`, which it does in place of opening the link. */
export function switchBackend(backend: string) {
  tellWindow(`silt://open/?backend=${encodeURIComponent(backend)}`)
}

/** `rgb(r, g, b)` or `rgba(…)` as a browser computes it, as `#rrggbb`. */
export function hexOf(computed: string): string | null {
  const channels = computed.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/)
  if (!channels) return null
  return `#${channels
    .slice(1, 4)
    .map((channel) => Number(channel).toString(16).padStart(2, '0'))
    .join('')}`
}

/** The link that tells the desktop app's window which theme the page shows (ADR 0099). */
export function themeLink(appearance: 'light' | 'dark', bg: string, fg: string): string {
  return `silt://theme/?${new URLSearchParams({ appearance, bg, fg })}`
}

let told = ''

const waiting: string[] = []

/** Sends the window a `silt://` link, one at a time: a second navigation in the same moment could cancel the first. */
function tellWindow(link: string) {
  waiting.push(link)
  if (waiting.length === 1) sendNext()
}

function sendNext() {
  const next = waiting[0]
  if (next === undefined) return
  window.location.href = next
  setTimeout(() => {
    waiting.shift()
    sendNext()
  }, 50)
}

/** Tells the desktop app's window the theme shown, so its frame and next start follow it; nothing in a browser. */
export function tellDesktopTheme(appearance: 'light' | 'dark') {
  if (!window.__siltBackends) return
  const body = getComputedStyle(document.body)
  const bg = hexOf(body.backgroundColor)
  const fg = hexOf(body.color)
  if (!bg || !fg) return
  const link = themeLink(appearance, bg, fg)
  if (link === told) return
  told = link
  tellWindow(link)
}

/** The commit the desktop app showing this page was built from, or none outside the app. */
export function desktopCommit(): string | undefined {
  return window.__siltDesktop?.commit || undefined
}

export function inDesktop(): boolean {
  return window.__siltDesktop !== undefined
}

/** Asks the desktop app to restart itself, on the build an update installed. */
export function relaunchDesktop() {
  window.location.href = 'silt://relaunch'
}

/** The link that tells the desktop app's window how far to zoom, in percent (ADR 0107). */
export function zoomLink(percent: number): string {
  return `silt://zoom/?${new URLSearchParams({ percent: String(percent) })}`
}

let toldZoom: number | undefined

/** Zooms the desktop app's window, which keeps the level for its next start; nothing in a browser, which has its own. */
export function tellDesktopZoom(percent: number) {
  if (!inDesktop() || percent === toldZoom) return
  toldZoom = percent
  tellWindow(zoomLink(percent))
}
