import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'

import ErrorBanner from '@/components/ErrorBanner.vue'
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

function mountOverview() {
  const wrapper = mount(OverviewView, {
    global: { plugins: [createPinia()], stubs: { TrendChart: true } },
  })
  wrappers.push(wrapper)
  return wrapper
}

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

    const wrapper = mountOverview()
    await flushPromises()
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

    const wrapper = mountOverview()
    await flushPromises()

    const row = wrapper.find('[data-testid="failure-row"]')
    expect(row.exists()).toBe(true)
    expect(row.text()).toContain('a.example.')
    const badge = row.find('[data-testid="analyzed-badge"]')
    expect(badge.exists()).toBe(true)
    expect(badge.text()).toBe('已分析')
  })
})
