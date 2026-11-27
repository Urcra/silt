import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from '@playwright/test'
import { allThreads, fixture, silt } from './fixture'

test('the prompt box undoes deleted text and restores its selection', async ({ page }) => {
  await page.goto(fixture().url)
  const prompt = page.getByRole('textbox', { name: 'first prompt' })
  await prompt.fill('Please fix the bug')
  await prompt.evaluate((element: HTMLTextAreaElement) => element.setSelectionRange(7, 15))
  await prompt.press('Backspace')
  await expect(prompt).toHaveValue('Please bug')
  await prompt.press('ControlOrMeta+z')
  await expect(prompt).toHaveValue('Please fix the bug')
  expect(await prompt.evaluate((element: HTMLTextAreaElement) => [element.selectionStart, element.selectionEnd])).toEqual([7, 15])
  await prompt.press('ControlOrMeta+Shift+Z')
  await expect(prompt).toHaveValue('Please bug')
  await prompt.press('ControlOrMeta+a')
  await prompt.press('Backspace')
  await prompt.press('ControlOrMeta+z')
  await expect(prompt).toHaveValue('Please bug')
})

test('a thread page shows what changes elsewhere without a reload', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await allThreads(page)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByText('Please add a farewell line to main')).toBeVisible()
  silt('comment', thread, '--entry', '0', 'said from a terminal')
  await expect(page.getByText(/: said from a terminal/)).toBeVisible()
})

test("the sidebar's and the changes panel's edges drag, and their widths are kept", async ({ page }) => {
  const { url, thread } = fixture()
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.goto(url)
  await allThreads(page)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toBeVisible()
  const sidebar = page.getByRole('complementary').first()
  const details = page.getByRole('complementary', { name: 'thread details' })
  const width = async (panel: typeof sidebar) => (await panel.boundingBox())?.width ?? 0
  // Level with a link, in moves past the browser's drag threshold: once the sidebar grows under where the press began,
  // the browser could drag the link instead, which cancels the pointer.
  const link = await sidebar.getByRole('link', { name: 'Inbox' }).boundingBox()
  const y = (link?.y ?? 200) + (link?.height ?? 0) / 2
  const drag = async (name: string, by: number) => {
    const box = await page.getByRole('separator', { name }).boundingBox()
    if (!box) throw new Error(`no ${name}`)
    await page.mouse.move(box.x + box.width / 2, y)
    await page.mouse.down()
    await page.mouse.move(box.x + box.width / 2 + by, y, { steps: 4 })
    await page.mouse.up()
  }
  const sidebarBefore = await width(sidebar)
  await drag('resize projects', 80)
  await expect.poll(() => width(sidebar)).toBe(sidebarBefore + 80)
  const detailsBefore = await width(details)
  await drag('resize thread details', -100)
  await expect.poll(() => width(details)).toBe(detailsBefore + 100)
  await page.getByRole('separator', { name: 'resize projects' }).press('ArrowLeft')
  await expect.poll(() => width(sidebar)).toBe(sidebarBefore + 64)

  await page.reload()
  await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toBeVisible()
  expect(await width(sidebar)).toBe(sidebarBefore + 64)
  expect(await width(details)).toBe(detailsBefore + 100)
  await expect(page.getByRole('separator', { name: 'resize projects' })).toHaveAttribute(
    'aria-valuenow',
    String(sidebarBefore + 64),
  )
})

test('lists threads and searches across compaction', async ({ page }) => {
  const { url, thread, fork } = fixture()
  await page.goto(url)
  await allThreads(page)
  const threads = page.getByTestId('threads')
  await expect(threads).toContainText(thread)
  await expect(threads.getByRole('row').filter({ hasText: fork })).toContainText(`fork of ${thread}`)
  // A thread's menu opens its history: its family in the graph, with its newest row chosen.
  await threads.getByRole('row').filter({ hasText: fork }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'History of this thread' }).click()
  await expect(page).toHaveURL(/\/graph\?thread=/)
  await expect(page.getByRole('tab', { name: 'This thread’s family' })).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('complementary', { name: 'selected' }).getByRole('link', { name: `Thread ${fork}` })).toBeVisible()
  await allThreads(page)

  await page.keyboard.press('ControlOrMeta+Shift+F')
  await page.getByLabel('search', { exact: true }).fill('farewell')
  await page.getByRole('main').getByRole('button', { name: 'Search', exact: true }).click()
  const results = page.getByTestId('search-results')
  // The index brackets each match, which the page marks.
  await expect(results.locator('mark').first()).toHaveText('farewell')
  // Hits group under their thread, and what came before the compaction says so.
  const inThread = results.getByRole('region', { name: 'hits in Add a farewell' })
  await expect(inThread.getByRole('link', { name: 'Add a farewell' })).toBeVisible()
  await expect(inThread.getByText('before compaction').first()).toBeVisible()

  await page.getByRole('button', { name: 'merged', exact: true }).click()
  await expect(page.getByRole('button', { name: 'clear status' })).toBeVisible()
  await page.getByRole('button', { name: 'clear status' }).click()
  await expect(page.getByRole('button', { name: 'clear status' })).toHaveCount(0)
})

test('shows a thread with compaction, turn diffs and comments', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await allThreads(page)
  await page.getByRole('link', { name: thread }).click()
  await expect(page.getByText('Please add a farewell line to main')).toBeVisible()
  await expect(page.getByTestId('compaction')).toBeVisible()
  await page.getByTestId('compaction').getByRole('button', { name: 'Show summary' }).click()
  await expect(page.getByText('Compaction summary: Summary: farewell was added to main.')).toBeVisible()
  // A right-click on it in the sidebar offers its id, for `silt resume` and `silt fork` in a terminal.
  await page
    .getByRole('navigation', { name: 'projects' })
    .getByRole('link', { name: /farewell/ })
    .first()
    .click({ button: 'right' })
  await expect(page.getByRole('menuitem', { name: `Copy thread ID ${thread.slice(-12)}` })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('menu')).toHaveCount(0)

  await page.getByRole('button', { name: 'show what turn 1 changed' }).click()
  await expect(page.getByTestId('diff').first()).toContainText('+ fn farewell() {}')
  // The changes are open beside the conversation (ADR 0044).
  const changes = page.getByRole('region', { name: 'changes' })
  await page.getByLabel('compare').selectOption({ label: 'Other…' })
  await page.getByLabel('diff base').selectOption('last-commit')
  await expect(changes.getByTestId('diff')).toContainText('+ fn farewell() {}')
  // Hiding git's changes leaves them out of the Changes tab's counts too, whatever the panel compares.
  const counted = page.waitForRequest((request) => /\/diff\/stats\?git=hide$/.test(request.url()))
  await page.getByRole('button', { name: 'How the diff shows' }).click()
  await page.getByRole('checkbox', { name: 'Hide git’s changes' }).check()
  await counted
  await page.getByRole('checkbox', { name: 'Hide git’s changes' }).uncheck()
  await page.keyboard.press('Escape')

  await page.getByText('Please add a farewell line to main').hover()
  await page.getByRole('button', { name: 'comment on entry 0' }).click()
  await page.getByLabel('comment', { exact: true }).fill('clear prompt')
  await page.getByRole('button', { name: 'Comment', exact: true }).click()
  await expect(page.getByText(/: clear prompt/)).toBeVisible()

  await page.getByLabel('compare').selectOption({ label: 'This thread' })
  await changes.getByRole('button', { name: 'comment on src/main.rs line 2' }).click()
  await page.getByLabel('code comment').fill('needs a doc comment')
  await page.getByRole('button', { name: 'Add single comment' }).click()
  await expect(changes.getByText(/: needs a doc comment/)).toBeVisible()
  // The inspector lists comments on code under Details.
  await page.getByRole('tab', { name: 'Details' }).click()
  const codeComments = page.getByTestId('code-comments')
  await expect(codeComments).toContainText('src/main.rs:2')
  await expect(codeComments).toContainText('needs a doc comment')
  await codeComments.getByRole('button', { name: 'reply' }).click()
  await codeComments.getByLabel('reply').fill('agreed, adding one')
  await codeComments.getByRole('button', { name: 'Reply', exact: true }).click()
  await expect(codeComments).toContainText(': agreed, adding one')
  // The reply shows under the comment in the diff too.
  await page.getByRole('tab', { name: /^Changes/ }).click()
  await expect(changes.getByText(/: agreed, adding one/)).toBeVisible()
  await page.getByRole('tab', { name: 'Details' }).click()
  await codeComments.getByRole('button', { name: 'resolve' }).click()
  await expect(codeComments.getByRole('button', { name: 'resolve' })).toHaveCount(0)
})

test("the palette finds threads and runs the page's actions, which keys run too", async ({ page }) => {
  const { url } = fixture()
  await page.goto(url)
  await page.keyboard.press('ControlOrMeta+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await palette.getByLabel('command').fill('farewell')
  await expect(palette.getByText('In history, including before compaction')).toBeVisible()
  await palette.getByRole('option', { name: /^Add a farewell/ }).click()
  await expect(page).toHaveURL(/\/t\//)
  // The thread's keys are its page's, which comes once its history has.
  await expect(page.getByTestId('history')).toBeVisible()

  // The changes start open beside the conversation; `d` closes and opens them.
  await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toBeVisible()
  await page.keyboard.press('d')
  await expect(page.getByRole('region', { name: 'changes' })).toHaveCount(0)
  await page.keyboard.press('d')
  await expect(page.getByRole('region', { name: 'changes' }).getByTestId('diff')).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByLabel('command').fill('against main')
  await page.keyboard.press('Enter')
  await expect(page.getByLabel('compare')).toHaveValue('main')

  await page.keyboard.press('n')
  await expect(page).toHaveURL(/:\d+\/(\?.*)?$/)
  // A key without a modifier is the person's while they type.
  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByLabel('command').pressSequentially('inbox')
  await expect(palette.getByLabel('command')).toHaveValue('inbox')
  await expect(page).toHaveURL(/:\d+\/(\?.*)?$/)
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/inbox$/)
})

test('reviews and approves a merge request', async ({ page }) => {
  const { url, thread } = fixture()
  await page.goto(url)
  await page.getByRole('link', { name: 'Merge requests' }).click()
  await page.getByTestId('merge-requests').getByRole('link', { name: 'Add forked' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('open')
  await expect(page.getByTestId('diff')).toContainText('+ fn forked() {}')
  await page.getByRole('tab', { name: /^The fork’s conversation/ }).click()
  await expect(page.getByRole('tabpanel', { name: "the fork's conversation" }).getByText('Continuing in a fork')).toBeVisible()
  await page.getByRole('tab', { name: /^Changes/ }).click()

  await page.getByRole('button', { name: 'Request changes' }).click()
  await page.getByLabel('reason').fill('add a test first')
  await page.getByRole('button', { name: 'Send the request' }).click()
  await expect(page.getByTestId('changes-requested')).toContainText('add a test first')
  await expect(page.getByTestId('mr-status')).toHaveText('open')

  await expect(page.getByTestId('seen-count')).toHaveText('0/1 seen')
  await page.getByLabel('seen src/main.rs').check()
  await expect(page.getByTestId('seen-count')).toHaveText('1/1 seen')
  await page.reload()
  await expect(page.getByLabel('seen src/main.rs')).toBeChecked()

  await page.getByRole('button', { name: 'Approve and merge' }).click()
  await expect(page.getByTestId('mr-status')).toHaveText('approved')
  // The request names the thread it merges into by title, linked to it.
  await page.locator(`main a[href$="${thread}"]`).first().click()
  await expect(page.getByText(/merged "Add forked" from fork/)).toBeVisible()
})

test('Settings › Memory shows what Claude Code remembers of a project, and edits and forgets it', async ({ page }) => {
  const { url, work } = fixture()
  const clone = join(work, 'repo')
  const folder = join(work, 'claude', 'projects', clone.replace(/[^A-Za-z0-9]/g, '-'), 'memory')
  mkdirSync(folder, { recursive: true })
  writeFileSync(
    join(folder, 'MEMORY.md'),
    '- [Tests use a real store](feedback-tests.md) — never mocks\n- [Release day](project-release.md) — Thursdays\n',
  )
  writeFileSync(
    join(folder, 'feedback-tests.md'),
    '---\nname: Tests use a real store\ndescription: Integration tests never mock the store\nmetadata:\n  type: feedback\n---\n\nStart daemons with a temporary SILT_HOME.\n',
  )
  writeFileSync(
    join(folder, 'project-release.md'),
    '---\nname: Release day\ndescription: Releases go out on Thursdays\nmetadata:\n  type: project\n---\n\nCut on Thursday.\n',
  )

  await page.goto(url)
  await page.goto(`${new URL(url).origin}/settings/memory`)
  const memory = page.getByRole('region', { name: 'Memory' })
  await expect(memory.getByText('2 memories', { exact: true })).toBeVisible()
  await memory.getByRole('link', { name: /repo/ }).click()

  const place = page.getByRole('region', { name: 'Memory of repo' })
  await place.getByRole('button', { name: 'Feedback 1' }).click()
  await expect(place.getByText('Release day')).toHaveCount(0)
  await place.getByRole('button', { name: 'All 2' }).click()
  await place.getByRole('button', { name: /Tests use a real store/ }).click()
  await expect(place.getByText('Start daemons with a temporary SILT_HOME.')).toBeVisible()

  await place.getByRole('button', { name: 'Edit' }).click()
  const editor = place.getByRole('textbox', { name: 'Edit feedback-tests.md' })
  await editor.fill(`${await editor.inputValue()}Never point one at the real one.\n`)
  await place.getByRole('button', { name: 'Save' }).click()
  await expect(place.getByText('Never point one at the real one.')).toBeVisible()
  expect(readFileSync(join(folder, 'feedback-tests.md'), 'utf8')).toContain('Never point one at the real one.')

  await place.getByRole('button', { name: 'Forget' }).click()
  await place.getByRole('button', { name: 'Forget' }).click()
  await expect(place.getByText('Tests use a real store')).toHaveCount(0)
  expect(existsSync(join(folder, 'feedback-tests.md'))).toBe(false)
  expect(readFileSync(join(folder, 'MEMORY.md'), 'utf8')).toBe('- [Release day](project-release.md) — Thursdays\n')
})

test('a memory moves from a project to everywhere and back', async ({ page }) => {
  const { url, work } = fixture()
  const clone = join(work, 'repo')
  const folder = join(work, 'claude', 'projects', clone.replace(/[^A-Za-z0-9]/g, '-'), 'memory')
  mkdirSync(folder, { recursive: true })
  writeFileSync(join(folder, 'MEMORY.md'), '- [Writes British English](user-british.md) — colour, behaviour\n')
  writeFileSync(
    join(folder, 'user-british.md'),
    '---\nname: Writes British English\ndescription: colour, behaviour\nmetadata:\n  type: feedback\n---\n\nColour, not color.\n',
  )

  await page.goto(url)
  await page.goto(`${new URL(url).origin}/settings/memory?memory=${encodeURIComponent(clone)}`)
  const place = page.getByRole('region', { name: 'Memory of repo' })
  await place.getByRole('button', { name: /Writes British English/ }).click()
  await place.getByRole('button', { name: 'Use everywhere' }).click()
  await expect(place.getByText('Writes British English')).toHaveCount(0)
  expect(existsSync(join(folder, 'user-british.md'))).toBe(false)

  await page.goto(`${new URL(url).origin}/settings/memory?memory=everywhere`)
  const everywhere = page.getByRole('region', { name: 'Memory of everywhere' })
  await everywhere.getByRole('button', { name: /Writes British English/ }).click()
  await expect(everywhere.getByText('Colour, not color.')).toBeVisible()
  await everywhere.getByRole('button', { name: 'Move to a project' }).click()
  await everywhere.getByLabel('Project to move it to').selectOption({ label: 'repo' })
  await expect(everywhere.getByText('Writes British English')).toHaveCount(0)
  expect(readFileSync(join(folder, 'user-british.md'), 'utf8')).toContain('Colour, not color.')
})
