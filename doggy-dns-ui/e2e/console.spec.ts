import { expect, test } from '@playwright/test'
import type { Page } from '@playwright/test'

test('overview renders live KPIs', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible()
  const success = page.locator('article').filter({ hasText: 'Success rate' })
  await expect(success).toBeVisible()
  await expect(success).not.toContainText('—')
})

test('copilot answers a suggested question', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '分析最近 1 小时失败' }).click()
  await expect(page.getByText('最近 1 小时整体成功率 99.7%')).toBeVisible({ timeout: 30_000 })
})

// The three cases below read their rows from the demo fixture
// (`src/datasource/fixture.rs`): eight groups aged 10 minutes to 8 days over
// dns-node-1/2/3 plus one host-less group, so every window asserted here has a
// row count the fixture actually answers with.
test('filters the failure list by host over a week', async ({ page }) => {
  await page.goto('/failures?windowMinutes=10080&instance=dns-node-2')

  // dns-node-2 owns exactly three groups inside a week: 10 minutes, 3 hours and
  // 6 days old. Pinning the count is what keeps a silently empty list from
  // passing this case.
  const rows = page.getByTestId('failure-row')
  await expect(rows).toHaveCount(3)
  for (const row of await rows.all()) {
    await expect(row.getByTestId('row-instance')).toHaveText('dns-node-2')
  }

  // One fixture group carries no host label at all, so the page has to say that
  // this filter does not reach it. The count is rows, not groups: the single
  // unlabeled group carries five telemetry rows.
  await expect(page.getByTestId('uncovered-notice')).toContainText('有 5 条遥测没有主机标签')
})

test('a one-hour bracket hides the three-day-old group', async ({ page }) => {
  await page.goto('/failures?windowMinutes=60')

  // The fixture's one-hour window keeps only the 10, 15 and 20-minute groups;
  // `api.internal.` is 3 days old, so it is the row that must be gone. Checked
  // per row: a locator of several rows would otherwise only ever read the first.
  const rows = page.getByTestId('failure-row')
  await expect(rows).toHaveCount(3)
  await expect(rows.filter({ hasText: 'api.internal.' })).toHaveCount(0)
})

test('the drawer opens from a row and keeps the bracket', async ({ page }) => {
  await page.goto('/failures?windowMinutes=10080')

  const rows = page.getByTestId('failure-row')
  await expect(rows).toHaveCount(6)
  await rows.first().click()

  // The trace id segment is part of the pattern on purpose: `windowMinutes=10080`
  // alone is also true of the list URL the click navigates *away* from, so the
  // assertion would settle on the pre-navigation page instead of the drawer.
  await expect(page).toHaveURL(/\/failures\/[0-9a-f]{16}\?windowMinutes=10080$/)
  await expect(page.getByTestId('range-chip')).toBeVisible()
  // The drawer is an overlay on the list's own route, so the rows stay mounted
  // behind it instead of being torn down and re-fetched.
  await expect(page.getByTestId('failure-row')).toHaveCount(6)
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('range-chip')).toHaveCount(0)
  await expect(page).toHaveURL(/\/failures\?windowMinutes=10080$/)
  await expect(page.getByTestId('failure-row')).toHaveCount(6)
})

test('the instance drawer counts failures over the chosen bracket', async ({ page }) => {
  // dns-node-2, not dns-node-1: node-1's only in-bracket failure group is 15
  // minutes old (its 45-minute group is NXDOMAIN — never a failure — and its
  // 9-row group is 8 days old, past the 7-day ceiling), so a week and an hour
  // both answer 12 and the number cannot move. node-2's three groups age 10
  // minutes, 3 hours and 6 days: an hour counts 2 rows, a week counts 13.
  const card = page.getByRole('button', { name: /^dns-node-2\b/ })

  await page.goto('/instances?windowMinutes=10080')
  await card.click()
  const count = page.getByTestId('failure-count')
  // A counted week is a number, never the em dash that means "the telemetry
  // source could not count at all".
  await expect(count).not.toHaveText('—')
  const week = Number(await count.innerText())

  await page.goto('/instances?windowMinutes=60')
  await card.click()
  const hour = Number(await page.getByTestId('failure-count').innerText())

  // The fixture spreads its groups from 10 minutes to 8 days, so a week counts
  // rows an hour cannot see. Asserting the number itself moves is the point:
  // a drawer that only *sends* the bracket and shows a constant would pass a
  // request-only assertion.
  expect(Number.isNaN(week)).toBe(false)
  expect(week).toBeGreaterThan(hour)
})

// D8 splits the two directions of the same rule, so each case gets its own
// 20s window (and its own timeout budget): a relative bracket must keep
// re-anchoring every 15s, an absolute one must stop asking entirely.
function countOverview(page: Page): string[] {
  const urls: string[] = []
  page.on('response', (response) => {
    if (new URL(response.url()).pathname === '/api/overview') urls.push(response.url())
  })
  return urls
}

test('a relative week refreshes the overview', async ({ page }) => {
  test.setTimeout(60_000)
  const urls = countOverview(page)

  await page.goto('/?windowMinutes=10080')
  await expect.poll(() => urls.length).toBeGreaterThan(0)
  expect(urls[0]).toContain('windowMinutes=10080')

  await page.waitForTimeout(20_000)
  // 15s polling means two samples inside 20s. Asserting ">1" rather than "==2"
  // keeps the case about the timer existing, not about wall-clock luck.
  expect(urls.length).toBeGreaterThan(1)
})

test('an absolute bracket is asked for once and never polled', async ({ page }) => {
  test.setTimeout(60_000)
  const to = Date.now()
  const from = to - 60 * 60 * 1000
  const urls = countOverview(page)

  await page.goto(`/?from=${from}&to=${to}`)
  await expect.poll(() => urls.length).toBeGreaterThan(0)
  expect(urls[0]).toContain(`from=${from}`)
  expect(urls[0]).toContain(`to=${to}`)

  await page.waitForTimeout(20_000)
  expect(urls).toHaveLength(1)
})

test('the shared time control keeps its labels on one line', async ({ page }) => {
  await page.goto('/?windowMinutes=10080')

  // The dashboard header is one nowrap flex row, and a CJK label's min-content
  // width is one character: while the control shipped five root nodes that row
  // squeezed "1 小时" into a 36x80 vertical stack and the hint into the middle of
  // it. Text assertions cannot see that, so this case reads geometry — a
  // single-line `text-xs` button is ~32px tall, a wrapped one is 80+.
  for (const testId of ['quick-1h', 'quick-6h', 'quick-24h', 'quick-7d']) {
    const box = await page.getByTestId(testId).boundingBox()
    expect(box, testId).not.toBeNull()
    expect(box!.height, testId).toBeLessThan(40)
  }

  // 起/止 sit inline with their inputs, so the bracket shares the presets' line
  // instead of growing a label row above itself. The row bottom-aligns, and an
  // input is shorter than a preset button, so inline leaves a 7.5px step; the
  // stacked label measured 25px because it added its own line on top.
  const presets = await page.getByTestId('quick-1h').boundingBox()
  const from = await page.getByTestId('range-from').boundingBox()
  expect(from).not.toBeNull()
  expect(from!.y - presets!.y).toBeLessThan(12)

  // Going back to a relative window is what the presets do; a dedicated reset
  // button next to them only repeated the first one.
  await expect(page.getByTestId('reset-time')).toHaveCount(0)

  // The hint owns a line of its own below the controls, not a column beside them.
  const hint = await page.getByTestId('range-hint').boundingBox()
  expect(hint).not.toBeNull()
  expect(hint!.y).toBeGreaterThan(presets!.y)
})
