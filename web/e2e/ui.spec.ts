import { expect, test } from '@playwright/test'
import { fixture, silt } from './fixture'

test('a thread page shows what changes elsewhere without a reload', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByText('Please add a farewell line to main')).toBeVisible()
  silt('comment', thread, '--entry', '0', 'said from a terminal')
  await expect(page.getByText(/: said from a terminal/)).toBeVisible()
})

test('lists threads and searches across compaction', async ({ page }) => {
  const { url, thread, fork } = fixture()
  await page.goto(url)
  const threads = page.getByTestId('threads')
  await expect(threads).toContainText(thread)
  await expect(threads.getByRole('row').filter({ hasText: fork })).toContainText(`fork of ${thread}`)

  await page.getByLabel('search', { exact: true }).fill('farewell')
  await page.getByRole('button', { name: 'Search' }).click()
  await expect(page.getByTestId('search-results')).toContainText('[farewell]')
})

test('shows a thread with compaction, turn diffs and comments', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByText('Please add a farewell line to main')).toBeVisible()
  await expect(page.getByTestId('compaction')).toBeVisible()
  await expect(page.getByText('Compaction summary: Summary: farewell was added to main.')).toBeVisible()
  await expect(page.getByText('Recorded with claude')).toBeVisible()
  await page.getByLabel('agent', { exact: true }).selectOption('codex')
  await expect(page.getByRole('button', { name: /^silt resume \w+ --agent codex$/ })).toBeVisible()

  await page.getByRole('button', { name: 'show what turn 1 changed' }).click()
  await expect(page.getByTestId('diff')).toContainText('+fn farewell() {}')
  await page.getByRole('button', { name: 'Show changes' }).click()
  const changes = page.getByRole('region', { name: 'changes' })
  await page.getByRole('tab', { name: 'Other…' }).click()
  await page.getByLabel('diff base').selectOption('last-commit')
  await expect(changes.getByTestId('diff')).toContainText('+fn farewell() {}')

  await page.getByText('Please add a farewell line to main').hover()
  await page.getByRole('button', { name: 'comment on entry 0' }).click()
  await page.getByLabel('comment', { exact: true }).fill('clear prompt')
  await page.getByRole('button', { name: 'Comment', exact: true }).click()
  await expect(page.getByText(/: clear prompt/)).toBeVisible()

  await page.getByRole('tab', { name: 'This thread' }).click()
  await changes.getByRole('button', { name: 'comment on src/main.rs line 2' }).click()
  await page.getByLabel('code comment').fill('needs a doc comment')
  await page.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(changes.getByText(/: needs a doc comment/)).toBeVisible()
  const codeComments = page.getByTestId('code-comments')
  await expect(codeComments).toContainText('src/main.rs:2')
  await expect(codeComments).toContainText('needs a doc comment')
  await codeComments.getByRole('button', { name: 'reply' }).click()
  await codeComments.getByLabel('reply').fill('agreed, adding one')
  await codeComments.getByRole('button', { name: 'Reply', exact: true }).click()
  await expect(codeComments).toContainText(': agreed, adding one')
  // The reply shows under the comment in the diff too.
  await expect(changes.getByText(/: agreed, adding one/)).toBeVisible()
  await codeComments.getByRole('button', { name: 'resolve' }).click()
  await expect(codeComments.getByRole('button', { name: 'resolve' })).toHaveCount(0)
})

test("the palette finds threads and runs the page's actions, which keys run too", async ({ page }) => {
  const { url } = fixture()
  await page.goto(url)
  await page.keyboard.press('ControlOrMeta+k')
  const palette = page.getByRole('dialog', { name: 'Go to or do' })
  await palette.getByLabel('command').fill('farewell')
  await expect(palette.getByRole('option').filter({ hasText: 'History' }).first()).toBeVisible()
  await palette.getByRole('option', { name: 'Threads Add a farewell' }).click()
  await expect(page).toHaveURL(/\/t\//)

  await page.keyboard.press('d')
  await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByLabel('command').fill('against main')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('tab', { name: 'Against main' })).toHaveAttribute('aria-selected', 'true')

  await page.keyboard.press('n')
  await expect(page).toHaveURL(/\/new$/)
  // A key without a modifier is the person's while they type.
  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByLabel('command').pressSequentially('inbox')
  await expect(palette.getByLabel('command')).toHaveValue('inbox')
  await expect(page).toHaveURL(/\/new$/)
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/inbox$/)
})

test('reviews and approves a merge request', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await page.getByRole('link', { name: 'Merge requests' }).click()
  await page.getByRole('link', { name: 'Add forked' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('open')
  await expect(page.getByTestId('diff')).toContainText('+fn forked() {}')
  await expect(page.getByRole('main').getByText('Continuing in a fork')).toBeVisible()

  await page.getByRole('button', { name: 'Approve and merge' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('approved')
  await page.getByRole('link', { name: thread }).first().click()
  await expect(page.getByText(/merged "Add forked" from fork/)).toBeVisible()
})
