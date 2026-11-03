import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, RouterView, type Router } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import FailuresView from '../FailuresView.vue'

const HOUR = 3_600_000

const ONE_GROUP = {
  key: 'trace:4bf9',
  traceId: '4bf92f3577b34da6',
  query: 'api.internal.',
  qtype: 'A',
  instance: 'dns-node-2',
  authority: 'forward[8.8.8.8:53]',
  responseCode: 'SERVFAIL',
  error: 'upstream timed out',
  count: 12,
  firstSeen: 1_761_433_200_000,
  lastSeen: 1_761_436_800_000,
  durationMs: 5012,
}

/**
 * Only the two failure endpoints are recorded: `useInstances()` arms its own
 * 15 s poll on mount, and counting that would make the "absolute bracket never
 * re-polls" case fail for a reason unrelated to the bracket. The composable's
 * own spec pins the interval itself.
 */
const fetches: string[] = []

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

interface Stubbed {
  rows?: unknown[]
  unlabeled?: number
  hosts?: string[]
}

function stubFetch({
  rows = [ONE_GROUP],
  unlabeled = 2,
  hosts = ['dns-node-1'],
}: Stubbed = {}): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.startsWith('/api/instances')) {
        return json(hosts.map((host) => ({ id: host, host, online: true })))
      }
      // `/api/failures/uncovered` is a prefix of the list route, so it is
      // matched first, and the markers call must not fall through to the rows.
      if (url.startsWith('/api/failures/uncovered')) {
        fetches.push(url)
        return json({ unlabeled })
      }
      if (url.startsWith('/api/analysis/markers')) return json([])
      if (url.startsWith('/api/failures?')) {
        fetches.push(url)
        return json(rows)
      }
      return json({ error: { code: 'unexpected_request', message: `unexpected ${url}` } }, 404)
    }),
  )
}

const DetailStub = defineComponent({
  name: 'DetailStub',
  template: '<div data-testid="detail-stub" />',
})

// The page renders its drawer through its own <RouterView/>, so the spec has to
// mount it the way AppShell does: mounting FailuresView directly would leave
// that RouterView at depth 0 and render the whole page a second time.
const TestRoot = defineComponent({
  name: 'TestRoot',
  components: { RouterView },
  template: '<RouterView />',
})

const wrappers: VueWrapper[] = []

async function mountView(initial: string): Promise<{ router: Router; wrapper: VueWrapper }> {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: '/failures',
        // The page's own draft watcher replaces by name, so the fixture has to
        // name the parent exactly like src/router/index.ts does.
        name: 'failures-list',
        component: FailuresView,
        children: [{ path: ':traceId', name: 'failure-detail', component: DetailStub }],
      },
    ],
  })
  await router.push(initial)
  await router.isReady()

  const wrapper = mount(TestRoot, { global: { plugins: [router, createPinia()] } })
  wrappers.push(wrapper)
  await flushPromises()
  return { router, wrapper }
}

beforeEach(() => {
  fetches.length = 0
  stubFetch()
})

afterEach(() => {
  // Unmounting stops the page's refresh intervals even when an assertion failed.
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('FailuresView', () => {
  it('writes the bracket into the URL when the filter changes', async () => {
    const { router, wrapper } = await mountView('/failures')

    await wrapper.find('[data-testid="quick-7d"]').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.query.windowMinutes).toBe('10080')
    expect(fetches.some((url) => url.includes('windowMinutes=10080'))).toBe(true)
  })

  it('reads the bracket from the URL on mount', async () => {
    const from = 1_761_429_600_000
    const to = 1_761_436_800_000
    await mountView(`/failures?from=${from}&to=${to}&instance=dns-node-2`)

    expect(fetches.some((url) => url.includes(`from=${from}`))).toBe(true)
    expect(fetches.some((url) => url.includes('instance=dns-node-2'))).toBe(true)
  })

  it('names an invalid bracket and does not fetch the list', async () => {
    const { wrapper } = await mountView('/failures?from=10&to=20')

    expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(true)
    expect(fetches.some((url) => url.includes('/api/failures?'))).toBe(false)
  })

  it('shows the uncovered notice with a way to clear the host filter', async () => {
    const { wrapper } = await mountView('/failures?windowMinutes=10080&instance=dns-node-2')

    const notice = wrapper.find('[data-testid="uncovered-notice"]')
    expect(notice.exists()).toBe(true)
    expect(notice.text()).toContain('2')
    expect(notice.text()).toContain('没有主机标签')

    await notice.find('button').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="uncovered-notice"]').exists()).toBe(false)
  })

  it('warns when the 200-group cap is hit', async () => {
    // The page always asks for 200 rows, so the cap is detected from the length of
    // the answer, not from the request.
    stubFetch({
      rows: Array.from({ length: 200 }, (_unused, index) => ({
        ...ONE_GROUP,
        key: `trace:t${index}`,
        traceId: `t${index}`,
      })),
      unlabeled: 0,
    })

    const { wrapper } = await mountView('/failures?windowMinutes=10080')

    expect(wrapper.find('[data-testid="cap-notice"]').text()).toContain('仅显示前')
  })

  it('stops auto-refreshing for an absolute bracket', async () => {
    vi.useFakeTimers()
    const now = Date.now()
    const { wrapper } = await mountView(`/failures?from=${now - 3 * HOUR}&to=${now}`)
    expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(false)

    fetches.length = 0
    vi.advanceTimersByTime(60_000)
    await flushPromises()

    expect(fetches).toHaveLength(0)
  })

  it('links a row to the detail route with the bracket preserved', async () => {
    const { router, wrapper } = await mountView('/failures?windowMinutes=10080')

    await wrapper.find('[data-testid="failure-row"]').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.name).toBe('failure-detail')
    expect(router.currentRoute.value.query.windowMinutes).toBe('10080')
    // The drawer only opens through the child route, so its mount is the proof
    // that the nesting works; the list stays rendered underneath it.
    expect(wrapper.find('[data-testid="detail-stub"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="failure-row"]').exists()).toBe(true)
  })

  it('titles the page with the bracket it is showing', async () => {
    const { wrapper } = await mountView('/failures?windowMinutes=10080')

    expect(wrapper.text()).toContain('失败列表')
    expect(wrapper.text()).toContain('近 7 天')
  })

  it('quotes both ceilings in the hint line', async () => {
    const { wrapper } = await mountView('/failures?windowMinutes=60')

    const hint = wrapper.find('[data-testid="range-hint"]')
    expect(hint.text()).toContain('5 分钟到 10080 分钟')
    expect(hint.text()).toContain('1440')
  })

  it('offers the discovered hosts, not just the ones on screen', async () => {
    stubFetch({ hosts: ['dns-node-1', 'dns-node-3'] })
    const { wrapper } = await mountView('/failures?windowMinutes=60')

    const options = wrapper
      .find('[data-testid="instance-select"]')
      .findAll('option')
      .map((option) => option.text())
    expect(options).toContain('dns-node-1')
    expect(options).toContain('dns-node-3')
  })

  it('escapes an absolute bracket back to the relative window', async () => {
    const now = Date.now()
    const { router, wrapper } = await mountView(`/failures?from=${now - 3 * HOUR}&to=${now}`)
    expect(router.currentRoute.value.query.from).toBeTruthy()

    await wrapper.find('[data-testid="reset-time"]').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.query.windowMinutes).toBe('60')
    expect(router.currentRoute.value.query.from).toBeUndefined()
  })
})
