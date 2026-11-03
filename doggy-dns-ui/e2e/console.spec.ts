import { expect, test } from '@playwright/test'

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
  const card = page.getByRole('button', { name: /^dns-node-1\b/ })

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
