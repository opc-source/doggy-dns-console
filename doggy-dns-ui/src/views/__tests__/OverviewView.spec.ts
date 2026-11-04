import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import ErrorBanner from '@/components/ErrorBanner.vue'
import FailureDrawer from '@/components/FailureDrawer.vue'
import { routes } from '@/router'
import OverviewView from '@/views/OverviewView.vue'

const HOUR = 3_600_000
const DAY = 24 * HOUR
// Pinned clock for the fake-timer cases below: `vi.setSystemTime` fixes
// `Date.now()` where the timer advance then moves it, so a window sitting on the
// 7-day ceiling cannot drift past validation mid-case.
const NOW = Date.parse('2026-11-04T12:00:00.000Z')

const overview = {
  windowMinutes: 60,
  instancesOnline: 3,
  instancesTotal: 4,
  queries: 128934,
  failures: 421,
  successRate: 0.997,
  p95LatencyMs: 12.4,
  series: [{ ts: 1_761_436_800_000, queries: 100, failures: 1 }],
}

const failures = [
  {
    key: 'trace:t1',
    traceId: 't1',
    query: 'a.example.',
    qtype: 'A',
    instance: 'dns-node-1',
    authority: 'forward',
    responseCode: 'SERVFAIL',
    error: 'timeout',
    count: 2,
    firstSeen: 1,
    lastSeen: 2,
    durationMs: 10,
  },
]

const markers = [
  {
    id: 'm1',
    traceId: 't1',
    status: 'succeeded',
    verdictCategory: 'upstream_timeout',
    confidence: 'high',
    attempt: 1,
    createdAt: 1,
    finishedAt: 2,
  },
]

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

// Recorded by `stubApi` so a case can count the requests the view actually made.
const fetches: string[] = []

function stubApi(overviewResponse: () => Response) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      fetches.push(url)
      if (url.includes('/api/overview')) return overviewResponse()
      if (url.includes('/api/analysis/markers')) return json(markers)
      return json(failures)
    }),
  )
}

const wrappers: VueWrapper[] = []

/**
 * The card's `see-all` is a RouterLink and a row click is a navigation, so the
 * view is mounted with the app's own route table: a hand-copied one would keep
 * passing after somebody renamed a route.
 */
async function mountOverview(
  path = '/',
): Promise<{ router: Router; wrapper: VueWrapper }> {
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()

  const wrapper = mount(OverviewView, {
    global: { plugins: [router, createPinia()], stubs: { TrendChart: true } },
  })
  wrappers.push(wrapper)
  await flushPromises()
  return { router, wrapper }
}

/**
 * The detail route is lazily imported, so the row click's navigation settles a
 * tick later than a plain flush; waiting on the route keeps the assertions below
 * about the query rather than about when the chunk happens to arrive. The wait is
 * shorter than vitest's own 5 s test timeout so a route that never arrives fails
 * with the route name in the message instead of a bare timeout.
 */
async function clickRow(wrapper: VueWrapper, router: Router) {
  await wrapper.find('[data-testid="failure-row"]').trigger('click')
  await vi.waitFor(() => expect(router.currentRoute.value.name).toBe('failure-detail'), {
    timeout: 3000,
  })
}

// jsdom never matches media queries, and the drawer reads one on mount: without
// this stub a drawer that leaked back into the overview would crash before the
// regression assertion could name it.
beforeEach(() => {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('1280px'),
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }))
})

describe('OverviewView', () => {
  afterEach(() => {
    // Unmounting stops the 15 s refresh interval even when an assertion failed.
    for (const wrapper of wrappers.splice(0)) wrapper.unmount()
    vi.unstubAllGlobals()
    // The week case below installs fake timers; reset them so a failure part-way
    // through that case cannot leak the frozen clock into the next one.
    vi.useRealTimers()
    fetches.length = 0
  })

  it('keeps stale data on error', async () => {
    let failOverview = false
    stubApi(() =>
      failOverview
        ? json({ error: { code: 'openobserve_unavailable', message: 'down' } }, 502)
        : json(overview),
    )

    const { wrapper } = await mountOverview()
    expect(wrapper.text()).toContain('99.7%')

    failOverview = true
    await wrapper.find('[aria-label="Refresh"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('99.7%')
    expect(wrapper.findComponent(ErrorBanner).exists()).toBe(true)
    expect(wrapper.text()).toContain('上次成功数据')
  })

  it('marks failures that already have an analysis', async () => {
    stubApi(() => json(overview))

    const { wrapper } = await mountOverview()

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.exists()).toBe(true)
    expect(row.text()).toContain('a.example.')
    const badge = row.find('[data-testid="analyzed-badge"]')
    expect(badge.exists()).toBe(true)
    expect(badge.text()).toBe('已有分析 · upstream_timeout')
  })

  it('routes a row click to the failure list with the window in the URL', async () => {
    stubApi(() => json(overview))
    const { router, wrapper } = await mountOverview()

    await clickRow(wrapper, router)

    expect(router.currentRoute.value.name).toBe('failure-detail')
    expect(router.currentRoute.value.params.traceId).toBeTruthy()
    expect(router.currentRoute.value.query.windowMinutes).toBe('60')
  })

  it('deep-links a badged row to the analysis its badge names', async () => {
    stubApi(() => json(overview))
    const { router, wrapper } = await mountOverview()

    await clickRow(wrapper, router)

    expect(router.currentRoute.value.query.analysis).toBe('m1')
  })

  it('sends the window the operator is looking at, not a fixed one', async () => {
    stubApi(() => json(overview))
    const { router, wrapper } = await mountOverview()

    await wrapper.get('[data-testid="quick-24h"]').trigger('click')
    await flushPromises()
    await clickRow(wrapper, router)

    // The card follows the picker, so the deep link investigates the same 24 h
    // the operator clicked inside; a hard-coded 60 would narrow it to an hour.
    expect(router.currentRoute.value.name).toBe('failure-detail')
    expect(router.currentRoute.value.query.windowMinutes).toBe('1440')
  })

  it('links the card header to the whole list', async () => {
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview()

    expect(wrapper.find('[data-testid="see-all"]').exists()).toBe(true)
  })

  it('carries the card window into the 查看全部 link', async () => {
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview()

    await wrapper.get('[data-testid="quick-6h"]').trigger('click')
    await flushPromises()

    // The rows are 6 h old; a bare /failures would silently show 60 minutes.
    const href = wrapper.find('[data-testid="see-all"]').attributes('href')
    expect(href).toMatch(/\/failures\?windowMinutes=360$/)
    expect(wrapper.get('[data-testid="quick-6h"]').attributes('aria-pressed')).toBe('true')
    expect(wrapper.get('[data-testid="quick-1h"]').attributes('aria-pressed')).toBe('false')
  })

  it('never mounts a drawer from the store', async () => {
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview()

    await wrapper.find('[data-testid="failure-row"]').trigger('click')
    await flushPromises()

    // The drawer is `failure-detail`'s alone: a second mount here is what lets a
    // pasted deep link and the browser's back button disagree with the screen.
    expect(wrapper.findComponent(FailureDrawer).exists()).toBe(false)
  })

  it('takes a week bracket from the URL and does not poll it', async () => {
    // The overview used to poll unconditionally; a 7-day bracket refreshed every
    // 15 s is a load test against OpenObserve, not a live view. Only an absolute
    // bracket stops the poll (design D8), so the week arrives as `from`/`to`: a
    // relative `windowMinutes=10080` would still re-anchor on every refresh.
    vi.useFakeTimers()
    const to = Date.now()
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview(`/?from=${to - 7 * DAY}&to=${to}`)

    const overviewCalls = fetches.filter((url) => url.includes('/api/overview'))
    expect(overviewCalls).toHaveLength(1)
    expect(overviewCalls[0]).toContain(`from=${to - 7 * DAY}`)
    expect(overviewCalls[0]).toContain(`to=${to}`)

    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.filter((url) => url.includes('/api/overview'))).toHaveLength(1)
    vi.useRealTimers()
    wrapper.unmount()
  })

  it('sends a relative week as windowMinutes and keeps polling it', async () => {
    // D8 stops the auto-refresh for an ABSOLUTE bracket only: a relative week has
    // to re-anchor on every poll, otherwise "近 7 天" freezes at the instant it was
    // opened. This is the unit-level guard against a "a week is a fixed bracket,
    // so don't poll it" confusion creeping into the relative path.
    vi.useFakeTimers()
    // A fixed clock, so the widest legal window cannot drift out of the ceiling
    // while the fake timers move `Date.now()` under the assertion.
    vi.setSystemTime(NOW)
    stubApi(() => json(overview))
    // 10080 is `MAX_RANGE_MINUTES` — a relative week, the widest window the
    // picker can ask for without an absolute `from`/`to`.
    const { wrapper } = await mountOverview('/?windowMinutes=10080')

    const overviewCalls = () => fetches.filter((url) => url.includes('/api/overview'))
    expect(overviewCalls()).toHaveLength(1)
    expect(overviewCalls()[0]).toContain('windowMinutes=10080')

    await vi.advanceTimersByTimeAsync(45_000)
    // 15 s × 3: the same re-anchoring the one-hour window already had.
    expect(overviewCalls()).toHaveLength(4)
    vi.useRealTimers()
    wrapper.unmount()
  })

  it('labels the header with the bracket a deep link describes', async () => {
    const to = Date.now()
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview(`/?from=${to - 3 * HOUR}&to=${to}`)

    // `rangeLabel` renders an absolute pair as `起 ~ 止`; a relative draft never
    // contains the tilde, so the copy itself proves the bracket came from the URL.
    expect(wrapper.get('[data-testid="range-label"]').text()).toContain('~')
    expect(fetches.some((url) => url.includes(`from=${to - 3 * HOUR}`))).toBe(true)
    wrapper.unmount()
  })
})
