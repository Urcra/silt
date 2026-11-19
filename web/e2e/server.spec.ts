import { type ChildProcess, execFileSync, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'

const SILT = join(import.meta.dirname, '..', '..', 'target', 'debug', 'silt')

let work: string
let server: ChildProcess
let provider: Server
let base: string

/** A stand-in OpenID Connect provider that signs in one person, Grace. */
function startProvider(): Promise<string> {
  const challenges = new Map<string, string>()
  const tokens = new Set<string>()
  provider = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    const json = (body: unknown, status = 200) => {
      response.writeHead(status, { 'content-type': 'application/json' })
      response.end(JSON.stringify(body))
    }
    if (url.pathname === '/.well-known/openid-configuration') {
      const issuer = `http://127.0.0.1:${(provider.address() as AddressInfo).port}`
      json({
        issuer,
        authorization_endpoint: `${issuer}/authorize`,
        token_endpoint: `${issuer}/token`,
        userinfo_endpoint: `${issuer}/userinfo`,
      })
    } else if (url.pathname === '/authorize') {
      const code = `code-${challenges.size}`
      challenges.set(code, url.searchParams.get('code_challenge') ?? '')
      const back = `${url.searchParams.get('redirect_uri')}?code=${code}&state=${encodeURIComponent(url.searchParams.get('state') ?? '')}`
      response.writeHead(200, { 'content-type': 'text/html' })
      response.end(`<h1>Example SSO</h1><a href="${back}">Sign in as Grace</a>`)
    } else if (url.pathname === '/token') {
      let body = ''
      request.on('data', (chunk: Buffer) => (body += chunk.toString()))
      request.on('end', () => {
        const form = new URLSearchParams(body)
        const verifier = form.get('code_verifier') ?? ''
        const challenge = createHash('sha256').update(verifier).digest('base64url')
        if (challenges.get(form.get('code') ?? '') !== challenge) return json({ error: 'invalid_grant' }, 400)
        const token = `token-${tokens.size}`
        tokens.add(token)
        json({ access_token: token, token_type: 'bearer' })
      })
    } else if (url.pathname === '/userinfo') {
      const token = (request.headers.authorization ?? '').replace('Bearer ', '')
      if (!tokens.has(token)) return json({}, 401)
      json({ sub: 'grace-1', preferred_username: 'grace', name: 'Grace H', email: 'grace@example.com', email_verified: true })
    } else {
      response.writeHead(404).end()
    }
  })
  return new Promise((resolve) =>
    provider.listen(0, '127.0.0.1', () => resolve(`http://127.0.0.1:${(provider.address() as AddressInfo).port}`)),
  )
}

async function freePort(): Promise<number> {
  const probe = createServer()
  await new Promise<void>((resolve) => probe.listen(0, '127.0.0.1', resolve))
  const port = (probe.address() as AddressInfo).port
  await new Promise((resolve) => probe.close(resolve))
  return port
}

test.beforeAll(async () => {
  work = mkdtempSync(join(tmpdir(), 'silt-server-ui-'))
  const issuer = await startProvider()
  const port = await freePort()
  base = `http://127.0.0.1:${port}`
  const data = join(work, 'data')
  execFileSync(SILT, ['server', 'init', '--data', data, '--listen', `127.0.0.1:${port}`, '--public-url', base], {
    env: { ...process.env, SILT_HOME: join(work, 'home') },
  })
  writeFileSync(
    join(data, 'server.toml'),
    `listen = "127.0.0.1:${port}"
public_url = "${base}"
[signup]
policy = "domains"
domains = ["example.com"]
[auth.oidc]
name = "Example SSO"
issuer = "${issuer}"
client_id = "silt"
client_secret = "shh"
`,
  )
  execFileSync(SILT, ['server', 'user', 'add', 'alice', '--password-stdin', '--data', data], {
    input: 'correct horse battery\n',
    env: { ...process.env, SILT_HOME: join(work, 'home') },
  })
  server = spawn(SILT, ['server', 'run', '--data', data], { stdio: 'ignore' })
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      await fetch(`${base}/login`)
      return
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
  }
  throw new Error('the server did not start')
})

test.afterAll(() => {
  server?.kill()
  provider?.close()
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
})

test('people sign in with a password', async ({ page }) => {
  await page.goto(`${base}/login`)
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('wrong password!')
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByText("That user name and password don't match.")).toBeVisible()
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByText('Signed in as alice')).toBeVisible()
})

test('the server hosts the web app for someone signed in', async ({ page }) => {
  await page.goto(`${base}/app/new`)
  await expect(page).toHaveURL(/\/login\?next=%2Fapp%2Fnew$/)
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Sign in' }).click()
  // Nothing here runs agents, so a new thread is All threads (ADR 0044).
  await expect(page).toHaveURL(`${base}/app/`)
  await expect(page.getByRole('link', { name: 'Silt' })).toBeVisible()
  await expect(
    page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'New thread', exact: true }),
  ).toHaveCount(0)
  await page.getByRole('navigation', { name: 'projects' }).getByRole('link', { name: 'All threads' }).click()
  await expect(page).toHaveURL(`${base}/app/threads`)
})

test('the hosted app lists the threads a device shared', async ({ page }) => {
  const mismatch = (await fetch(`${base}/v1/listing`).then((response) => response.json())) as { supported: number[] }
  const headers = { 'content-type': 'application/json', 'silt-protocol': String(mismatch.supported[0]) }
  const login = await fetch(`${base}/v1/login/password`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ user: 'alice', password: 'correct horse battery', name: 'laptop' }),
  }).then((response) => response.json())
  const thread = '0192ab0000007000800012345678abcd'
  const created = await fetch(`${base}/v1/threads`, {
    method: 'POST',
    headers: { ...headers, authorization: `Bearer ${login.token}` },
    body: JSON.stringify({
      id: thread,
      family: '0192ab0000007000800012345678abce',
      repo: 'github.com/acme/hosted',
      created_ms: Date.now(),
      lease: null,
      owner: '',
      collaborators: [],
    }),
  })
  expect(created.status).toBe(204)

  await page.goto(`${base}/app/`)
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page).toHaveURL(`${base}/app/`)
  await expect(page.getByRole('link', { name: /untitled/i }).first()).toBeVisible()
  await page
    .getByRole('link', { name: /untitled/i })
    .first()
    .click()
  await expect(page).toHaveURL(new RegExp(`/app/t/${thread}`))

  // The service worker that shows pushes lives under the app, as JavaScript.
  const worker = await page.request.get(`${base}/app/sw.js`)
  expect(worker.headers()['content-type']).toContain('javascript')
  const key = (await (await page.request.get(`${base}/app/api/push/key`)).json()) as string
  expect(key.length).toBeGreaterThan(80)
})

test('the hosted app keeps text and layout in the browser and sizes the page by it (ADR 0102)', async ({ page }) => {
  await page.goto(`${base}/app/settings/text`)
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Sign in' }).click()
  const section = page.getByRole('region', { name: 'Text and layout' })
  await expect(section.getByText('kept in this browser', { exact: true })).toBeVisible()
  // A browser zooms on its own, so only the desktop app offers it.
  await expect(section.getByRole('group', { name: 'zoom' })).toHaveCount(0)
  await section.getByRole('button', { name: 'bigger conversation text' }).click()
  await section.getByRole('button', { name: 'bigger conversation text' }).click()
  const chat = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--chat-size').trim())
  await expect.poll(chat).toBe('15px')
  await page.reload()
  await expect.poll(chat).toBe('15px')
  await expect(
    page.getByRole('region', { name: 'Text and layout' }).getByRole('button', { name: 'reset conversation text' }),
  ).toBeVisible()
  const css = (await (
    await page.request.get(`${base}/app/api/themes/css?text=${encodeURIComponent('{"code_size":40}')}`)
  ).json()) as {
    css: string
  }
  expect(css.css).toContain(':root:root:root{--code-size:20px;}')
})

test('a device is approved in the browser', async ({ page }) => {
  // Asked without a version, the server says which one it speaks.
  const mismatch = (await fetch(`${base}/v1/device/start`, { method: 'POST' }).then((response) => response.json())) as {
    supported: number[]
  }
  const protocol = String(mismatch.supported[0])
  const api = (path: string, body: unknown) =>
    fetch(`${base}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'silt-protocol': protocol },
      body: JSON.stringify(body),
    }).then((response) => response.json())
  const started = await api('/v1/device/start', { name: 'build-box' })
  expect(await api('/v1/device/poll', { device_code: started.device_code })).toBe('pending')

  await page.goto(started.verification_url)
  await page.getByLabel('User').fill('alice')
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByText('Sign in build-box as alice?')).toBeVisible()
  await expect(page.getByText(started.user_code)).toBeVisible()
  await page.getByRole('button', { name: 'Approve' }).click()
  await expect(page.getByText('the device is signed in')).toBeVisible()

  const approved = await api('/v1/device/poll', { device_code: started.device_code })
  expect(approved.approved.login.user).toBe('alice')
})

test('people sign in through the identity provider', async ({ page }) => {
  await page.goto(`${base}/login`)
  await page.getByRole('link', { name: 'Continue with Example SSO' }).click()
  await expect(page.getByRole('heading', { name: 'Example SSO' })).toBeVisible()
  await page.getByRole('link', { name: 'Sign in as Grace' }).click()
  await expect(page.getByText('Signed in as grace')).toBeVisible()
})
