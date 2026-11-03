import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import ErrorBanner from '@/components/ErrorBanner.vue'
import FailureDrawer from '@/components/FailureDrawer.vue'
import { routes } from '@/router'
import OverviewView from '@/views/OverviewView.vue'

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

function stubApi(overviewResponse: () => Response) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
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
async function mountOverview(): Promise<{ router: Router; wrapper: VueWrapper }> {
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push('/')
  await router.isReady()

  const wrapper = mount(OverviewView, {
    global: { plugins: [router, createPinia()], stubs: { TrendChart: true } },
  })
  wrappers.push(wrapper)
  await flushPromises()
  return { router, wrapper }
}

/** The window chips are plain buttons labelled `1h`/`6h`/`24h` with no test id. */
function chipByText(wrapper: VueWrapper, label: string) {
  const [chip] = wrapper.findAll('button').filter((button) => button.text() === label)
  if (!chip) throw new Error(`no ${label} window chip`)
  return chip
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
    expect(badge.text()).toBe('已有分析')
  })

  it('routes a row click to the failure list with the window in the URL', async () => {
    stubApi(() => json(overview))
    const { router, wrapper } = await mountOverview()

    await clickRow(wrapper, router)

    expect(router.currentRoute.value.name).toBe('failure-detail')
    expect(router.currentRoute.value.params.traceId).toBeTruthy()
    expect(router.currentRoute.value.query.windowMinutes).toBe('60')
  })

  it('sends the window the operator is looking at, not a fixed one', async () => {
    stubApi(() => json(overview))
    const { router, wrapper } = await mountOverview()

    await chipByText(wrapper, '24h').trigger('click')
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

  it('never mounts a drawer from the store', async () => {
    stubApi(() => json(overview))
    const { wrapper } = await mountOverview()

    await wrapper.find('[data-testid="failure-row"]').trigger('click')
    await flushPromises()

    // The drawer is `failure-detail`'s alone: a second mount here is what lets a
    // pasted deep link and the browser's back button disagree with the screen.
    expect(wrapper.findComponent(FailureDrawer).exists()).toBe(false)
  })
})
