import { expect, test } from '@playwright/test'
import { fixture } from './fixture'

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

  await page.getByRole('button', { name: 'show what turn 1 changed' }).click()
  await expect(page.getByTestId('diff')).toContainText('+fn farewell() {}')

  await page.getByText('Please add a farewell line to main').hover()
  await page.getByRole('button', { name: 'comment on entry 0' }).click()
  await page.getByLabel('comment', { exact: true }).fill('clear prompt')
  await page.getByRole('button', { name: 'Comment', exact: true }).click()
  await expect(page.getByText(/: clear prompt/)).toBeVisible()

  await page.getByLabel('file').fill('src/main.rs')
  await page.getByLabel('lines').fill('2')
  await page.getByLabel('code comment').fill('needs a doc comment')
  await page.getByRole('button', { name: 'Add' }).click()
  const codeComments = page.getByTestId('code-comments')
  await expect(codeComments).toContainText('src/main.rs:2')
  await expect(codeComments).toContainText('needs a doc comment')
  await codeComments.getByRole('button', { name: 'resolve' }).click()
  await expect(codeComments.getByRole('button', { name: 'resolve' })).toHaveCount(0)
})

test('reviews and approves a merge request', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await page.getByRole('link', { name: 'Merge requests' }).click()
  await page.getByRole('link', { name: 'Add forked' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('open')
  await expect(page.getByTestId('diff')).toContainText('+fn forked() {}')
  await expect(page.getByText('Continuing in a fork')).toBeVisible()

  await page.getByRole('button', { name: 'Approve and merge' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('approved')
  await page.getByRole('link', { name: thread }).first().click()
  await expect(page.getByText(/merged "Add forked" from fork/)).toBeVisible()
})
