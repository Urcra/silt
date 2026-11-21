// Phones on the local network, with no server (ADR 0112): turning it on, the certificate step, pairing a phone
// that the person allows here, and the phone's own pages, each kept as a screen reference.

import { createServer } from 'node:net'
import { type Browser, type Page, chromium, expect, test } from '@playwright/test'
import { type Fixture, SILT, runFixture, teardown } from './fixture'

const NAME = 'workstation'

let world: Fixture
let port: number
let phoneBrowser: Browser

/** A free port with a free one after it: the listener's and the certificate page's. */
async function freePorts(): Promise<number> {
  const listen = (wanted: number) =>
    new Promise<number | null>((resolve) => {
      const server = createServer()
      server.once('error', () => resolve(null))
      server.listen(wanted, '0.0.0.0', () => {
        const address = server.address()
        server.close(() => resolve(typeof address === 'object' && address ? address.port : null))
      })
    })
  for (;;) {
    const first = await listen(0)
    if (first && first < 65535 && (await listen(first + 1))) return first
  }
}

test.beforeAll(async () => {
  port = await freePorts()
  world = JSON.parse(
    runFixture('fixture.sh', {
      ...process.env,
      SILT_LAN_PORT: String(port),
      SILT_LAN_ADDRESSES: '192.168.1.23',
      SILT_LAN_NAME: NAME,
      // Whose Silt it is shows on the phone's pages; the same on every machine taking the references.
      USER: 'ada',
      USERNAME: 'ada',
    }) || '{}',
  ) as Fixture
  // The phone reaches the machine's `.local` name on loopback, and skips the certificate warning that installing
  // the machine's authority would make go away.
  phoneBrowser = await chromium.launch({
    executablePath: process.env.SILT_E2E_CHROMIUM,
    args: ['--host-resolver-rules=MAP *.local 127.0.0.1', '--proxy-server=direct://'],
  })
})

test.afterAll(async () => {
  await phoneBrowser?.close()
  teardown(SILT, world.work, [world.siltHome])
})

const VARIANTS = [
  { name: 'desktop-light', width: 1440, height: 900, colorScheme: 'light' },
  { name: 'desktop-dark', width: 1440, height: 900, colorScheme: 'dark' },
  { name: 'phone-light', width: 390, height: 844, colorScheme: 'light' },
  { name: 'phone-dark', width: 390, height: 844, colorScheme: 'dark' },
] as const

function shotOptions(page: Page) {
  return {
    mask: [page.locator('[data-volatile]'), page.getByText(/\/tmp\/silt-|127\.0\.0\.1:\d+/).filter({ visible: true })],
    animations: 'disabled' as const,
    caret: 'hide' as const,
    maxDiffPixelRatio: 0.001,
  }
}

/** Takes the card in every variant, scrolled into view. */
async function shoot(page: Page, name: string, variants: readonly (typeof VARIANTS)[number][] = VARIANTS) {
  for (const variant of variants) {
    await page.setViewportSize({ width: variant.width, height: variant.height })
    await page.emulateMedia({ colorScheme: variant.colorScheme, reducedMotion: 'reduce' })
    const card = page.getByRole('region', { name: 'phones on this network' })
    if ((await card.count()) > 0) await card.scrollIntoViewIfNeeded()
    await expect(page).toHaveScreenshot(`${name}-${variant.name}.png`, shotOptions(page))
  }
}

test('a phone pairs with this machine over the network, once allowed here', async ({ page }) => {
  const origin = new URL(world.url).origin
  await page.clock.setFixedTime(Date.now())
  await page.goto(world.url)
  await page.goto(`${origin}/settings/devices`)
  const card = page.getByRole('region', { name: 'phones on this network' })
  await expect(card.getByRole('button', { name: 'Turn on' })).toBeVisible()
  await shoot(page, 'lan-off')

  await card.getByRole('button', { name: 'Turn on' }).click()
  await expect(card.getByText('Trust this machine')).toBeVisible()
  // `silt-workstation-<suffix>.local`: the suffix is made once per machine.
  const host = await page.evaluate(async () => {
    const response = await fetch('/api/lan', { headers: { Authorization: `Bearer ${sessionStorage.getItem('silt-token')}` } })
    return ((await response.json()) as { host: string }).host
  })
  expect(host).toMatch(new RegExp(`^silt-${NAME}-[0-9a-f]{4}\\.local$`))
  await expect(card.getByTestId('ca-fingerprint')).toBeVisible()
  await shoot(page, 'lan-trust')

  // The phone gets the certificate's page over plain HTTP, with the same fingerprint.
  const fingerprint = (await card.getByTestId('ca-fingerprint').innerText()).replace(/\s+/g, '')
  const phone = await phoneBrowser.newContext({
    viewport: { width: 390, height: 844 },
    isMobile: true,
    hasTouch: true,
    ignoreHTTPSErrors: true,
    userAgent:
      'Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1',
  })
  const onPhone = await phone.newPage()
  await onPhone.goto(`http://127.0.0.1:${port + 1}/trust`)
  expect((await onPhone.getByTestId('fingerprint').innerText()).replace(/\s+/g, '')).toBe(fingerprint)
  for (const scheme of ['light', 'dark'] as const) {
    await onPhone.emulateMedia({ colorScheme: scheme })
    await expect(onPhone).toHaveScreenshot(`lan-phone-trust-${scheme}.png`, shotOptions(onPhone))
  }

  await card.getByRole('button', { name: 'Next: pair the phone' }).click()
  const code = (await card.getByTestId('lan-pairing-code').innerText()).replace('-', '')
  await shoot(page, 'lan-pair')

  // The phone opens the code and waits; the machine asks.
  await onPhone.goto(`https://${host}:${port}/pair/${code}`)
  await expect(onPhone.getByText('Waiting for you to allow this phone')).toBeVisible()
  for (const scheme of ['light', 'dark'] as const) {
    await onPhone.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' })
    await expect(onPhone).toHaveScreenshot(`lan-phone-waiting-${scheme}.png`, shotOptions(onPhone))
  }
  const ask = card.getByRole('region', { name: 'a phone asks to pair' })
  await expect(ask).toBeVisible()
  await expect(ask.getByText('iPhone · Safari 19')).toBeVisible()
  await shoot(page, 'lan-ask')

  // Elsewhere in the app, the same ask shows as a prompt.
  await page.goto(`${origin}/t/${world.thread}`)
  const prompt = page.getByRole('alertdialog', { name: 'a phone asks to pair' })
  await expect(prompt).toBeVisible()
  await page.goto(`${origin}/settings/devices`)
  await expect(prompt).toBeHidden()

  await ask.getByRole('button', { name: 'Allow' }).click()
  await expect(onPhone.getByText('This phone can now answer permissions and review merge requests')).toBeVisible({
    timeout: 10_000,
  })
  for (const scheme of ['light', 'dark'] as const) {
    await onPhone.emulateMedia({ colorScheme: scheme })
    await expect(onPhone).toHaveScreenshot(`lan-phone-paired-${scheme}.png`, shotOptions(onPhone))
  }

  // Paired, the phone opens the app itself, and is asked about notifications.
  await onPhone.getByRole('link', { name: 'Open Silt' }).click()
  await expect(onPhone.getByRole('region', { name: 'notifications' })).toBeVisible()
  await expect(onPhone.getByRole('region', { name: 'waiting on you' }).getByText('Add forked')).toBeVisible()
  for (const scheme of ['light', 'dark'] as const) {
    await onPhone.emulateMedia({ colorScheme: scheme })
    await expect(onPhone).toHaveScreenshot(`lan-phone-needs-you-${scheme}.png`, shotOptions(onPhone))
  }

  // The machine lists it, and the phone may not change the machine.
  await page.reload()
  await expect(page.getByRole('region', { name: 'iPhone' })).toBeVisible()
  await expect(page.getByText('this network', { exact: true })).toBeVisible()
  await shoot(page, 'lan-paired')
  const refused = await onPhone.evaluate(async () => {
    const response = await fetch('/api/lan', {
      method: 'PUT',
      headers: { 'X-Silt-Request': '1', 'Content-Type': 'application/json' },
      body: '{"on":false}',
    })
    return response.status
  })
  expect(refused).toBe(403)

  // Signed out on the machine, the phone is told to pair again.
  await page.getByRole('region', { name: 'iPhone' }).getByRole('button', { name: 'Sign out' }).click()
  await onPhone.reload()
  await expect(onPhone.getByRole('heading', { name: 'Pair this phone' })).toBeVisible()

  // The code was used once; opening it again finds it used up.
  await phone.clearCookies()
  await onPhone.goto(`https://${host}:${port}/pair/${code}`)
  await expect(onPhone.getByText('This code is used up or has expired.')).toBeVisible()
  await phone.close()
})
