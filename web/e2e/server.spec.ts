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
  return new Promise((resolve) => provider.listen(0, '127.0.0.1', () => resolve(`http://127.0.0.1:${(provider.address() as AddressInfo).port}`)))
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
  rmSync(work, { recursive: true, force: true })
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

test('a device is approved in the browser', async ({ page }) => {
  const api = (path: string, body: unknown) =>
    fetch(`${base}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'silt-protocol': '2' },
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
