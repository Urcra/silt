// Phones on the local network, with no server (ADR 0112): the card on Devices that turns it on, trusts and pairs;
// the prompt that asks about a phone wherever the person is; and, on the phone, turning notifications on.

import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { type ReactNode, useEffect, useState } from 'react'
import { api, type Capabilities, type LanAsk, type LanView } from '../api'
import { useNow } from '../now'
import { notificationsAllowed, pushable, subscribeToPush, workerRefused } from '../notifier'
import { keys, message } from '../query'
import { useLan, useLanMe } from '../lan'
import { onLan } from '../served'
import { Banner, Button, Chip, Dot, Icon, KeyValue, Skeleton, buttonClass, cx, toast } from '../ui'

function useLanChange() {
  const queries = useQueryClient()
  return (view: LanView) => queries.setQueryData(keys.lan, view)
}

const PAIR_CAPABILITIES: { id: keyof Capabilities; label: string }[] = [
  { id: 'answer_permissions', label: 'Answer agents’ permission requests' },
  { id: 'review', label: 'Approve and decline merge requests' },
  { id: 'send_prompts', label: 'Send prompts and start threads' },
  { id: 'run_agents', label: 'Start and stop agents' },
]

const PHONE: Capabilities = {
  answer_permissions: true,
  send_prompts: false,
  run_agents: false,
  review: true,
  sample: false,
  page_tools: false,
}

/** `a, b and c`. */
function listed(items: string[]): string {
  return items.length < 2 ? (items[0] ?? '') : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`
}

function Qr({ svg, label }: { svg: string; label: string }) {
  return (
    <img
      alt={label}
      data-volatile
      className="h-40 w-40 shrink-0 rounded-md border border-line bg-[var(--paper-50)] p-1.5"
      src={`data:image/svg+xml;utf8,${encodeURIComponent(svg)}`}
    />
  )
}

/** The fingerprint as a phone shows it, eight pairs to a line. */
function Fingerprint({ text }: { text: string }) {
  const pairs = text.split(' ')
  const lines = Array.from({ length: Math.ceil(pairs.length / 8) }, (_, line) => pairs.slice(line * 8, line * 8 + 8).join(' '))
  return (
    <p className="font-mono text-2xs tracking-[0.04em] text-fg-1" data-testid="ca-fingerprint" data-volatile>
      {lines.map((line) => (
        <span key={line} className="block">
          {line}
        </span>
      ))}
    </p>
  )
}

function Steps({ current, skipped = false }: { current: 1 | 2; skipped?: boolean }) {
  const step = (number: 1 | 2, label: string) => {
    const done = number < current
    const here = number === current
    const passed = done && skipped
    return (
      <li
        className={cx('flex items-center gap-1.5', here ? 'font-medium text-fg-1' : 'text-fg-2')}
        aria-current={here ? 'step' : undefined}
      >
        <span
          className={cx(
            'inline-flex h-5 w-5 items-center justify-center rounded-full font-mono text-2xs',
            here ? 'bg-accent text-on-accent' : done && !passed ? 'bg-ok-tint text-ok' : 'bg-surface-2 text-fg-2',
          )}
        >
          {passed ? '–' : done ? <Icon name="check" className="h-3 w-3" /> : number}
        </span>
        {label}
        {passed && <span className="text-fg-3">skipped</span>}
      </li>
    )
  }
  return (
    <ol aria-label="steps" className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
      {step(1, 'Trust this machine')}
      <li aria-hidden="true" className="text-fg-3">
        ›
      </li>
      {step(2, 'Pair the phone')}
    </ol>
  )
}

function Address({ view }: { view: LanView }) {
  return (
    <KeyValue
      rows={[
        ['Address', <span key="address" data-volatile>{`${view.host}:${view.port}`}</span>],
        ...(view.addresses.length > 0
          ? ([
              [
                'Also at',
                <span key="also" data-volatile>
                  {view.addresses.map((address) => `${address}:${view.port}`).join(', ')}
                </span>,
              ],
            ] as [string, ReactNode][])
          : []),
      ]}
    />
  )
}

/** A phone asking to pair, with Allow and Deny. */
export function AskCard({ ask, onAnswered }: { ask: LanAsk; onAnswered: (view: LanView) => void }) {
  const answer = (allow: boolean) =>
    api.lanAnswer(ask.id, allow).then(
      (view) => {
        toast(allow ? `${ask.phone.split(' · ')[0]} is paired` : 'Denied', allow ? 'ok' : 'info')
        onAnswered(view)
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const may = listed(
    PAIR_CAPABILITIES.filter((capability) => ask.capabilities[capability.id]).map((capability) => capability.label.toLowerCase()),
  )
  return (
    <section aria-label="a phone asks to pair" className="flex flex-col gap-3 rounded-md border border-accent bg-accent-tint p-3">
      <div className="flex items-center gap-2">
        <Icon name="phone" className="h-4 w-4 text-accent-fg" />
        <h3 className="text-sm font-medium">A phone asks to pair</h3>
      </div>
      <KeyValue
        rows={[
          ['Phone', ask.phone],
          ['From', ask.from],
          [
            'Code',
            <span key="code" data-volatile>
              {ask.code}
            </span>,
          ],
        ]}
      />
      <p className="text-xs text-fg-2">Allow it only if it’s your phone, in your hand. It will {may || 'only read'}.</p>
      <div className="flex flex-wrap gap-2">
        <Button variant="primary" onClick={() => void answer(true)}>
          Allow
        </Button>
        <Button variant="danger" onClick={() => void answer(false)}>
          Deny
        </Button>
      </div>
    </section>
  )
}

/** The name the machine's authority installs as on a phone: `Silt on workstation (ada)`. */
function Installed({ view }: { view: LanView }) {
  return (
    <>
      Silt on {view.machine}
      {view.person && (
        <>
          {' ('}
          <span data-volatile>{view.person}</span>)
        </>
      )}
    </>
  )
}

const WITHOUT_CERTIFICATE = 'silt-lan-without-certificate'

/** Whether the person pairs phones without the machine's certificate, as they last chose here (ADR 0113). */
function useWithoutCertificate(): [boolean, (without: boolean) => void] {
  const [without, setWithout] = useState(() => {
    try {
      return localStorage.getItem(WITHOUT_CERTIFICATE) === '1'
    } catch {
      return false
    }
  })
  const choose = (chosen: boolean) => {
    try {
      if (chosen) localStorage.setItem(WITHOUT_CERTIFICATE, '1')
      else localStorage.removeItem(WITHOUT_CERTIFICATE)
    } catch {
      // Kept for this page only.
    }
    setWithout(chosen)
  }
  return [without, choose]
}

function WithoutCertificate() {
  return (
    <Banner>
      <div className="flex flex-col gap-1.5 text-xs leading-normal text-fg-1">
        <p>The phone will warn that the connection isn’t private. Go on past the warning once, then pair.</p>
        <p>Without the certificate, the phone:</p>
        <ul className="flex list-disc flex-col gap-1 pl-4">
          <li>gets no notifications when an agent needs you;</li>
          <li>can’t add Silt to its Home Screen as an app;</li>
          <li>may show the warning again after a while.</li>
        </ul>
        <p className="text-fg-2">Install the certificate later to get these back. The phone stays paired.</p>
      </div>
    </Banner>
  )
}

function Trust({
  view,
  without,
  onWithout,
  onNext,
}: {
  view: LanView
  without: boolean
  onWithout: (without: boolean) => void
  onNext: () => void
}) {
  const [platform, setPlatform] = useState<'apple' | 'android'>('apple')
  const next = (
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="primary" onClick={onNext}>
        Next: pair the phone
      </Button>
      {!without && <span className="text-xs text-fg-2">Trusted it before? Go straight on.</span>}
    </div>
  )
  return (
    <>
      <Address view={view} />
      <Steps current={1} />
      <label className="flex items-center gap-1.5 text-xs max-md:min-h-11">
        <input
          type="checkbox"
          checked={without}
          className="accent-[var(--accent)]"
          onChange={(event) => onWithout(event.target.checked)}
        />
        Pair without installing the certificate
      </label>
      {without ? (
        <>
          <WithoutCertificate />
          {next}
        </>
      ) : (
        <>
          <div className="flex flex-wrap items-start gap-4">
            {view.trust_qr_svg && <Qr svg={view.trust_qr_svg} label={`QR code to get ${view.machine}'s certificate`} />}
            <div className="flex flex-[1_1_220px] flex-col gap-2 text-xs leading-normal">
              <p>
                Scan with the phone’s camera to get {view.machine}’s certificate
                {view.trust_url && (
                  <>
                    , or open{' '}
                    <span className="font-mono break-all" data-volatile>
                      {view.trust_url}
                    </span>
                  </>
                )}
                . The phone should show this fingerprint:
              </p>
              {view.fingerprint && <Fingerprint text={view.fingerprint} />}
              <p className="text-fg-2">If it doesn’t match, don’t install it: something on this network is in the way.</p>
            </div>
          </div>
          <div role="group" aria-label="phone" className="flex gap-1.5">
            <Chip selected={platform === 'apple'} onClick={() => setPlatform('apple')}>
              iPhone
            </Chip>
            <Chip selected={platform === 'android'} onClick={() => setPlatform('android')}>
              Android
            </Chip>
          </div>
          <ol className="flex list-decimal flex-col gap-1 pl-4 text-xs leading-normal">
            {platform === 'apple' ? (
              <>
                <li>
                  Allow the download, then in Settings open <b className="font-medium">Profile Downloaded</b> and install{' '}
                  <b className="font-medium">
                    <Installed view={view} />
                  </b>
                  .
                </li>
                <li>
                  In Settings › General › About › Certificate Trust Settings, turn on{' '}
                  <b className="font-medium">
                    <Installed view={view} />
                  </b>
                  .
                </li>
              </>
            ) : (
              <>
                <li>
                  Open the downloaded file. If Android only offers to save it, go to Settings › Security › More security settings
                  › Encryption &amp; credentials › Install a certificate › CA certificate, and pick it.
                </li>
                <li>
                  Name it{' '}
                  <b className="font-medium">
                    <Installed view={view} />
                  </b>
                  . Android warns about network monitoring for any certificate you add; this one can only vouch for {view.machine}
                  .
                </li>
              </>
            )}
          </ol>
          {next}
        </>
      )}
    </>
  )
}

function Pair({
  view,
  without,
  onBack,
  onChanged,
}: {
  view: LanView
  without: boolean
  onBack: () => void
  onChanged: (view: LanView) => void
}) {
  const now = useNow(1000)
  const [byAddress, setByAddress] = useState(false)
  const pairing = view.pairing
  if (!pairing) return <Skeleton className="h-40" />
  const left = Math.max(0, Math.round((Number(pairing.expires_ms) - now) / 1000))
  const address = byAddress && pairing.address_url && pairing.address_qr_svg
  const url = address ? pairing.address_url! : pairing.url
  const repair = (capabilities: Capabilities) =>
    api.lanPair(capabilities).then(onChanged, (failure: unknown) => toast(message(failure), 'danger'))
  return (
    <>
      <Address view={view} />
      <Steps current={2} skipped={without} />
      <div className="flex flex-wrap items-start gap-4">
        <Qr svg={address ? pairing.address_qr_svg! : pairing.qr_svg} label="QR code to pair a phone" />
        <ol className="flex flex-[1_1_180px] list-decimal flex-col gap-1 pl-4 text-xs leading-normal">
          <li>
            Scan with the phone’s camera, or open{' '}
            <span className="font-mono break-all" data-volatile>
              {url.replace(/\/[^/]+$/, '')}
            </span>{' '}
            and type the code.
          </li>
          {without && (
            <li>
              The phone warns that the connection isn’t private. On an iPhone, choose <b className="font-medium">Show Details</b>,
              then <b className="font-medium">visit this website</b>. On Android, choose <b className="font-medium">Advanced</b>,
              then <b className="font-medium">Proceed</b>.
            </li>
          )}
          <li>Allow the phone here when it asks.</li>
        </ol>
      </div>
      <div className="flex flex-wrap items-baseline gap-3">
        <span className="font-mono text-lg tracking-[0.12em]" data-testid="lan-pairing-code" data-volatile>
          {pairing.code}
        </span>
        <span className="text-xs text-fg-2" data-volatile>
          {left > 0 ? `expires in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}` : 'expired'}
        </span>
        {left === 0 ? (
          <Button size="sm" className="ml-auto" onClick={() => void repair(pairing.capabilities)}>
            New code
          </Button>
        ) : (
          pairing.address_url && (
            <Button variant="ghost" size="sm" className="ml-auto" onClick={() => setByAddress(!byAddress)}>
              {byAddress ? 'Use the name instead' : 'Use the address instead'}
            </Button>
          )
        )}
      </div>
      {byAddress && (
        <p className="text-2xs text-fg-2">
          An address can change when the router restarts; the phone would then need pairing again. The name doesn’t.
        </p>
      )}
      {without && view.fingerprint && (
        <details className="group/check text-xs">
          <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 [&::-webkit-details-marker]:hidden">
            <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/check:rotate-90" />
            Check it’s really {view.machine}
          </summary>
          <div className="mt-2 flex flex-col gap-1.5 leading-normal text-fg-2">
            <p>Before going on, open the warning’s certificate details. Its SHA-256 fingerprint should be:</p>
            <Fingerprint text={view.fingerprint} />
            <p>If it isn’t, don’t go on: something on this network is in the way.</p>
          </div>
        </details>
      )}
      <fieldset className="flex flex-col gap-1.5 text-xs">
        <legend className="mb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">The phone may</legend>
        {PAIR_CAPABILITIES.map((capability) => (
          <label key={capability.id} className="flex items-center gap-1.5 max-md:min-h-11">
            <input
              type="checkbox"
              checked={pairing.capabilities[capability.id]}
              className="accent-[var(--accent)]"
              onChange={(event) => void repair({ ...pairing.capabilities, [capability.id]: event.target.checked })}
            />
            {capability.label}
          </label>
        ))}
      </fieldset>
      <details className="group/help text-xs">
        <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 [&::-webkit-details-marker]:hidden">
          <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/help:rotate-90" />
          The phone can’t open it?
        </summary>
        <ul className="mt-2 flex list-disc flex-col gap-1.5 pl-4 leading-normal text-fg-2">
          <li>Check the phone is on the same Wi-Fi as {view.machine}, not on mobile data or a guest network.</li>
          <li>
            Some networks don’t pass <span className="font-mono">.local</span> names. Use the address instead.
          </li>
          <li>
            {without
              ? 'Without the certificate, the phone always warns first. Go on past the warning, or install the certificate: go back to the first step.'
              : `A certificate warning means the phone doesn’t trust ${view.machine} yet. Go back to the first step.`}
          </li>
          {view.firewall && (
            <li>
              A firewall may be blocking port {view.port}:{' '}
              {view.firewall.startsWith('sudo') ? <code className="font-mono">{view.firewall}</code> : view.firewall}
            </li>
          )}
        </ul>
      </details>
      <div className="flex gap-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          Back
        </Button>
      </div>
    </>
  )
}

function Summary({ view, onPair, onTrust }: { view: LanView; onPair: () => void; onTrust: () => void }) {
  const queries = useQueryClient()
  const forget = () =>
    api.lanForget().then(
      (forgotten) => {
        queries.setQueryData(keys.lan, forgotten)
        toast('Every phone is signed out, with a new certificate', 'ok')
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const renews = view.renews_ms
    ? new Date(Number(view.renews_ms)).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
    : null
  return (
    <>
      <Address view={view} />
      <KeyValue
        rows={[
          [
            'Certificate',
            <span key="certificate" data-volatile>
              {view.fingerprint
                ? `${view.fingerprint
                    .replace(/ /g, '')
                    .slice(0, 16)
                    .replace(/(.{4})/g, '$1 ')}…`
                : '—'}
            </span>,
          ],
          ...(renews
            ? ([
                [
                  'Renews',
                  <span key="renews" data-volatile>
                    by itself, {renews}
                  </span>,
                ],
              ] as [string, ReactNode][])
            : []),
        ]}
      />
      <p className="text-xs text-fg-2">
        Phones reach {view.machine} only while they’re on this network. Away from it, they wait until they’re back.
      </p>
      <div className="flex flex-wrap gap-2">
        <Button onClick={onPair}>Pair a phone</Button>
        <Button variant="ghost" onClick={onTrust}>
          Show the certificate step
        </Button>
      </div>
      <details className="group/more text-xs">
        <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 [&::-webkit-details-marker]:hidden">
          <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/more:rotate-90" />
          Start over
        </summary>
        <div className="mt-2 flex flex-col gap-2 text-fg-2">
          <p>Makes a new certificate and signs every phone out. Each needs the new certificate and pairing again.</p>
          <div>
            <Button variant="danger" size="sm" onClick={() => void forget()}>
              Forget phones and certificate
            </Button>
          </div>
        </div>
      </details>
    </>
  )
}

type Step = 'summary' | 'trust' | 'pair'

/** Phones on this network: turning it on, the certificate step, pairing, and how it stands (ADR 0112). */
export function LanCard() {
  const lan = useLan()
  const changed = useLanChange()
  const [step, setStep] = useState<Step>('summary')
  const [without, setWithout] = useWithoutCertificate()
  const turn = useMutation({
    mutationFn: (on: boolean) => api.lanTurn(on),
    onSuccess: (view) => {
      changed(view)
      setStep(view.state === 'on' && view.phones.length === 0 ? 'trust' : 'summary')
    },
    onError: (failure: unknown) => toast(message(failure), 'danger'),
  })
  const pair = () =>
    api.lanPair(lan.data?.pairing?.capabilities ?? PHONE).then(
      (view) => {
        changed(view)
        setStep('pair')
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const view = lan.data
  const on = view && (view.state === 'on' || view.state === 'no_network')
  const head = (
    <div className="flex items-center gap-2">
      <h2 className="text-sm font-medium">Phones on this network</h2>
      {turn.isPending ? (
        <span className="ml-auto inline-flex items-center gap-1.5 text-xs">
          <Dot tone="accent" />
          {turn.variables ? 'turning on' : 'turning off'}
        </span>
      ) : on ? (
        <>
          <span className="ml-auto inline-flex items-center gap-1.5 text-xs">
            <Dot tone="ok" />
            on
          </span>
          <Button variant="ghost" size="sm" onClick={() => turn.mutate(false)}>
            Turn off
          </Button>
        </>
      ) : (
        view &&
        view.state !== 'off' && (
          <span className="ml-auto inline-flex items-center gap-1.5 text-xs">
            <Dot tone="danger" />
            off
          </span>
        )
      )}
    </div>
  )
  let body
  if (lan.isPending) body = <Skeleton className="h-24" />
  else if (lan.error || !view) body = <p className="text-sm text-danger">{message(lan.error)}</p>
  else if (turn.isPending && turn.variables)
    body = (
      <div className="flex flex-wrap items-start gap-4">
        <div className="h-40 w-40 shrink-0 animate-pulse rounded-md bg-surface-2" />
        <div className="flex flex-[1_1_160px] flex-col gap-2 text-xs text-fg-2">
          <p>Making {view.machine}’s certificate…</p>
          <p>Opening port {view.port} on this network…</p>
        </div>
      </div>
    )
  else if (view.state === 'off')
    body = (
      <>
        <p className="text-sm text-fg-2">
          Answer agents and review from your phone while it’s on the same Wi-Fi as {view.machine}. No server needed.
        </p>
        <ul className="flex list-disc flex-col gap-1 pl-4 text-xs leading-normal text-fg-2">
          <li>Only phones you allow here get in, and each may do only what you give it.</li>
          <li>The connection is encrypted with a certificate {view.machine} makes. Each phone trusts it once.</li>
        </ul>
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="primary" onClick={() => turn.mutate(true)}>
            Turn on
          </Button>
          <span className="text-xs text-fg-2">
            or run <code className="font-mono">silt lan on</code>
          </span>
        </div>
      </>
    )
  else if (view.state === 'port_taken' || view.state === 'failed')
    body = (
      <>
        <Banner tone="danger" urgent>
          {view.state === 'port_taken' ? (
            <>
              <p>Another program is using port {view.port}.</p>
              <p className="mt-1 text-xs">
                Stop it, or set <code className="font-mono">SILT_LAN_PORT</code> to a free port and restart Silt’s daemon with{' '}
                <code className="font-mono">silt daemon restart</code>. Phones already paired will need the new address.
              </p>
            </>
          ) : (
            <>
              <p>Phones can’t reach {view.machine}.</p>
              <p className="mt-1 text-xs">{view.error}</p>
            </>
          )}
        </Banner>
        <div className="flex gap-2">
          <Button onClick={() => turn.mutate(true)}>Try again</Button>
        </div>
      </>
    )
  else if (view.asks.length > 0)
    body = (
      <>
        <Address view={view} />
        <AskCard
          ask={view.asks[0]}
          onAnswered={(answered) => {
            changed(answered)
            setStep('summary')
          }}
        />
      </>
    )
  else if (view.state === 'no_network' && step === 'summary')
    body = (
      <>
        <Banner tone="warn">
          <p>{view.machine} isn’t on a local network.</p>
          <p className="mt-1 text-xs">Connect it to Wi-Fi or Ethernet. It turns on by itself once it has an address.</p>
        </Banner>
        <Summary view={view} onPair={() => void pair()} onTrust={() => setStep('trust')} />
      </>
    )
  else if (step === 'trust') body = <Trust view={view} without={without} onWithout={setWithout} onNext={() => void pair()} />
  else if (step === 'pair' && view.pairing)
    body = <Pair view={view} without={without} onBack={() => setStep('trust')} onChanged={changed} />
  else body = <Summary view={view} onPair={() => setStep('trust')} onTrust={() => setStep('trust')} />
  return (
    <section aria-label="phones on this network" className="flex flex-col gap-3 rounded-md border border-line bg-elevated p-4">
      {head}
      {body}
    </section>
  )
}

/** A phone asking to pair while the person is on another page: a prompt that stays until answered. */
export function LanAskPrompt() {
  const lan = useLan()
  const changed = useLanChange()
  const navigate = useNavigate()
  const path = useRouterState({ select: (state) => state.location.pathname })
  const ask = lan.data?.asks?.[0]
  if (!ask || path.endsWith('/settings/devices')) return null
  return (
    <div
      role="alertdialog"
      aria-label="a phone asks to pair"
      className="fixed right-4 bottom-4 z-40 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2 rounded-md border border-line bg-elevated p-3 text-sm shadow-[var(--shadow-2)] max-md:bottom-20"
    >
      <div className="flex items-center gap-2">
        <Icon name="phone" className="h-4 w-4 text-accent-fg" />
        <span className="font-medium">A phone asks to pair</span>
      </div>
      <p className="text-xs text-fg-2">
        {ask.phone}, from {ask.from}, code {ask.code}
      </p>
      <div className="flex gap-2">
        <Button
          variant="primary"
          size="sm"
          onClick={() =>
            void api.lanAnswer(ask.id, true).then(
              (view) => {
                changed(view)
                toast(`${ask.phone.split(' · ')[0]} is paired`, 'ok')
              },
              (failure: unknown) => toast(message(failure), 'danger'),
            )
          }
        >
          Allow
        </Button>
        <Button
          variant="danger"
          size="sm"
          onClick={() => void api.lanAnswer(ask.id, false).then(changed, (failure: unknown) => toast(message(failure), 'danger'))}
        >
          Deny
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto"
          onClick={() => void navigate({ to: '/settings/$section', params: { section: 'devices' } })}
        >
          Show
        </Button>
      </div>
    </div>
  )
}

const DISMISSED = 'silt-lan-notify-dismissed'

/** On a phone paired over the network: asks to turn notifications on, once, until it's on or put off. */
export function LanNotifyBanner() {
  const me = useLanMe()
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISSED) === '1'
    } catch {
      return false
    }
  })
  const [on, setOn] = useState(false)
  const [refused, setRefused] = useState(false)
  useEffect(() => {
    if (onLan()) void workerRefused().then(setRefused)
  }, [])
  if (!onLan() || !me.data || me.data.notifies || on || dismissed) return null
  if (refused) return <NotifyNeedsCertificate machine={me.data.machine} onDismiss={() => setDismissed(dismiss())} />
  if (notificationsAllowed()) return null
  const installable = !pushable()
  return (
    <section
      aria-label="notifications"
      className="mx-4 mt-3 flex flex-col gap-2 rounded-md border border-line bg-elevated p-3 text-sm"
    >
      <div className="flex items-center gap-2">
        <Icon name="bell" className="h-4 w-4 text-accent-fg" />
        <span className="font-medium">Hear when an agent needs you</span>
      </div>
      <p className="text-xs text-fg-2">
        {installable
          ? 'Add Silt to your Home Screen first: Share › Add to Home Screen. Then open it from there to turn notifications on.'
          : `${me.data.machine} notifies this phone when an agent waits on a permission or a plan, or stops with an error, while it’s on this network.`}
      </p>
      <div className="flex gap-2">
        {!installable && (
          <Button
            variant="primary"
            onClick={() =>
              void Notification.requestPermission()
                .then(() => subscribeToPush())
                .then(
                  () => setOn(notificationsAllowed()),
                  (failure: unknown) => toast(message(failure), 'danger'),
                )
            }
          >
            Turn on notifications
          </Button>
        )}
        <Button variant="ghost" onClick={() => setDismissed(dismiss())}>
          Not now
        </Button>
      </div>
    </section>
  )
}

function dismiss(): true {
  try {
    localStorage.setItem(DISMISSED, '1')
  } catch {
    // Kept for this page only.
  }
  return true
}

/** The certificate page, served over plain HTTP on the port after this one (ADR 0112). */
function trustUrl(): string {
  return `http://${location.hostname}:${Number(location.port || 443) + 1}/trust`
}

/** On a phone paired without the machine's certificate, whose browser refuses the service worker notifications need. */
function NotifyNeedsCertificate({ machine, onDismiss }: { machine: string; onDismiss: () => void }) {
  return (
    <section
      aria-label="notifications"
      className="mx-4 mt-3 flex flex-col gap-2 rounded-md border border-line bg-elevated p-3 text-sm"
    >
      <div className="flex items-center gap-2">
        <Icon name="bell" className="h-4 w-4 text-accent-fg" />
        <span className="font-medium">Hear when an agent needs you</span>
      </div>
      <p className="text-xs text-fg-2">
        This phone was paired without {machine}’s certificate, and notifications need it. Install it once, then come back here to
        turn them on. The phone stays paired.
      </p>
      <div className="flex gap-2">
        <a className={buttonClass('primary')} href={trustUrl()}>
          Get the certificate
        </a>
        <Button variant="ghost" onClick={onDismiss}>
          Not now
        </Button>
      </div>
    </section>
  )
}

/** On a phone that isn't paired, or no longer is: what to do, in place of a page of errors. */
export function LanUnpaired() {
  return (
    <div className="mx-auto flex max-w-[400px] flex-col gap-4 px-4 pt-[12vh]">
      <span className="text-display font-medium tracking-[-0.04em]">silt</span>
      <h1 className="text-lg font-medium">Pair this phone</h1>
      <p className="text-sm text-fg-2">
        This phone isn’t paired with this machine, or was signed out. On the machine, open Settings › Devices and sync, then Pair
        a phone under Phones on this network, and scan the code.
      </p>
      <a className="text-sm text-accent-fg" href="/pair">
        I have a code
      </a>
    </div>
  )
}
