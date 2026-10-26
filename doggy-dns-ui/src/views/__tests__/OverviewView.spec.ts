import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { describe, expect, it, vi } from 'vitest'

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

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

describe('OverviewView', () => {
  it('keeps stale data on error', async () => {
    let failOverview = false
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input)
        if (url.includes('/api/overview')) {
          return failOverview
            ? json({ error: { code: 'openobserve_unavailable', message: 'down' } }, 502)
            : json(overview)
        }
        return json(failures)
      }),
    )

    const wrapper = mount(OverviewView, {
      global: { plugins: [createPinia()], stubs: { TrendChart: true } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('99.7%')

    failOverview = true
    await wrapper.find('[aria-label="Refresh"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('99.7%')
    expect(wrapper.findComponent(ErrorBanner).exists()).toBe(true)
    expect(wrapper.text()).toContain('上次成功数据')
  })
})
