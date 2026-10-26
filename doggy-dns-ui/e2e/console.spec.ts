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

test('failure drawer opens and closes', async ({ page }) => {
  await page.goto('/')
  await page.getByTestId('failure-row').first().click()
  await expect(page.getByTestId('failure-drawer')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('failure-drawer')).toBeHidden()
})
