import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, RouterView, type Router } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import FailuresView from '../FailuresView.vue'
import { msToDatetime } from '@/lib/failureFilter'

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

/** The newest recorded failure-list request, parsed, so a test names the wire params. */
function latestListRequest(): URLSearchParams {
  let found: string | undefined
  for (const url of fetches) if (url.startsWith('/api/failures?')) found = url
  if (found === undefined) throw new Error('the page made no failure-list request')
  return new URLSearchParams(found.slice(found.indexOf('?') + 1))
}

/** The bar's two date inputs, in template order: 起 then 止. */
function dateInput(wrapper: VueWrapper, index: number) {
  const input = wrapper.findAll('input[type="datetime-local"]')[index]
  if (input === undefined) throw new Error('the filter bar has no date input')
  return input
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

    // `invalid-range` is the page's banner and the page's alone: the drawer's own
    // refusal carries `drawer-invalid-range`, so opening the detail route on a bad
    // bracket can never add a second one of these ids under /failures.
    expect(wrapper.findAll('[data-testid="invalid-range"]')).toHaveLength(1)
    expect(fetches.some((url) => url.includes('/api/failures?'))).toBe(false)
    // No rows were fetched, so a cap notice would be a claim about nothing.
    expect(wrapper.find('[data-testid="cap-notice"]').exists()).toBe(false)
  })

  it('refuses a hand-edited one-sided bracket without querying the list', async () => {
    const { wrapper } = await mountView(`/failures?from=${Date.now() - 3 * HOUR}`)

    const banner = wrapper.findAll('[data-testid="invalid-range"]')
    expect(banner).toHaveLength(1)
    expect(banner[0]!.text()).toMatch(/起止/)
    expect(fetches.some((url) => url.includes('/api/failures?'))).toBe(false)
    expect(fetches.some((url) => url.includes('/uncovered'))).toBe(false)
  })

  it('shows the uncovered notice with a way to clear the host filter', async () => {
    const { router, wrapper } = await mountView('/failures?windowMinutes=10080&instance=dns-node-2')

    const notice = wrapper.find('[data-testid="uncovered-notice"]')
    expect(notice.exists()).toBe(true)
    expect(notice.attributes('role')).toBe('status')
    expect(notice.text()).toContain('2')
    expect(notice.text()).toContain('没有主机标签')

    await notice.find('button').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="uncovered-notice"]').exists()).toBe(false)
    // The count was bracket-scoped, so the action drops the host and keeps the
    // bracket: the rows it just counted are the rows this window shows.
    expect(router.currentRoute.value.query).toEqual({ windowMinutes: '10080' })
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

    // PAGE_LIMIT is one constant with two visible consequences: the request asks
    // for 200 and the notice quotes the same 200.
    expect(fetches.some((url) => url.includes('limit=200'))).toBe(true)
    const capNotice = wrapper.find('[data-testid="cap-notice"]')
    expect(capNotice.text()).toContain('仅显示前 200 组')
    expect(capNotice.attributes('role')).toBe('status')
  })

  it('leaves the refusal to the drawer on the detail route', async () => {
    const { wrapper } = await mountView('/failures/t1?from=10&to=20')

    // The drawer owns the refusal on this route (its `drawer-invalid-range` is
    // pinned in FailureView.spec.ts); the page's own two banners would be the
    // second refusal for one URL, so both stay away.
    expect(wrapper.find('[data-testid="detail-stub"]').exists()).toBe(true)
    expect(wrapper.findAll('[data-testid="invalid-range"]')).toHaveLength(0)
    expect(wrapper.text()).not.toContain('区间不足 5 分钟')
  })

  it('stops auto-refreshing for an absolute bracket', async () => {
    vi.useFakeTimers()
    const now = Date.now()
    const { wrapper } = await mountView(`/failures?from=${now - 3 * HOUR}&to=${now}`)
    expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(false)

    // Positive control: the mount itself did ask, so the zero below proves the poll
    // stayed asleep rather than that nothing was ever recorded.
    expect(fetches.some((url) => url.includes('/api/failures?'))).toBe(true)
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
    // The page owns the full-field row, not the overview card's dense one: the
    // host column only exists in full mode.
    expect(wrapper.find('[data-testid="row-instance"]').text()).toBe('dns-node-2')
  })

  it('quotes the one ceiling and names the windowless endpoint', async () => {
    const { wrapper } = await mountView('/failures?windowMinutes=60')

    const hint = wrapper.find('[data-testid="range-hint"]')
    expect(hint.text()).toContain('5 分钟到 10080 分钟')
    expect(hint.text()).not.toContain('1440')
    expect(hint.text()).toContain('/api/instances')
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

  it('completes the bracket when only 起 is typed', async () => {
    const { router, wrapper } = await mountView('/failures?windowMinutes=10080')
    fetches.length = 0

    const before = Date.now()
    const fromText = msToDatetime(before - 2 * HOUR)
    await dateInput(wrapper, 0).setValue(fromText)
    await flushPromises()

    // One side is never enough: `toRequestQuery` and `toRouteQuery` emit `from`/`to`
    // only as a pair, so the bar completes the bracket at the point of creation.
    const params = latestListRequest()
    const fromMs = Date.parse(fromText)
    expect(params.get('from')).toBe(String(fromMs))
    // The relative window goes with the pair: the request never asks for both.
    expect(params.get('windowMinutes')).toBeNull()
    expect(params.get('to')).not.toBeNull()
    // The missing end is the bar's own now.
    const to = Number(params.get('to'))
    expect(to).toBeGreaterThanOrEqual(before)
    expect(to).toBeLessThanOrEqual(Date.now())
    // And the URL carries exactly the pair the request did.
    expect(router.currentRoute.value.query).toEqual({ from: String(fromMs), to: String(to) })
  })

  it('completes the bracket when only 止 is typed', async () => {
    const { wrapper } = await mountView('/failures?windowMinutes=10080')
    fetches.length = 0

    const toText = msToDatetime(Date.now() - 3 * 24 * HOUR)
    await dateInput(wrapper, 1).setValue(toText)
    await flushPromises()

    const params = latestListRequest()
    const toMs = Date.parse(toText)
    expect(params.get('to')).toBe(String(toMs))
    expect(params.get('windowMinutes')).toBeNull()
    // The missing start is the page's own default window measured back from the
    // typed end — anchoring it on now would put the start after the end for any
    // past date, and the page would refuse its own input.
    expect(Number(params.get('from'))).toBe(toMs - HOUR)
  })

  it('drops the whole bracket when a date input is cleared', async () => {
    const now = Date.now()
    const { router, wrapper } = await mountView(`/failures?from=${now - 3 * HOUR}&to=${now}`)
    expect(latestListRequest().get('from')).toBe(String(now - 3 * HOUR))
    fetches.length = 0

    await dateInput(wrapper, 0).setValue('')
    await flushPromises()

    // Chosen behaviour for the cleared case: an empty input is not a half bracket
    // to complete, so both sides go and the default relative window takes over —
    // the URL never falls back to the server's own default.
    const params = latestListRequest()
    expect(params.get('from')).toBeNull()
    expect(params.get('to')).toBeNull()
    expect(params.get('windowMinutes')).toBe('60')
    expect(router.currentRoute.value.query).toEqual({ windowMinutes: '60' })
  })

  it('follows the URL when it changes after mount', async () => {
    const { router } = await mountView('/failures?windowMinutes=60')
    fetches.length = 0

    await router.push('/failures?windowMinutes=10080')
    await flushPromises()
    expect(latestListRequest().get('windowMinutes')).toBe('10080')

    // The absolute pair travels the same way: this is the path a shared deep link
    // or the browser's back button takes, and the draft has to follow the URL.
    const from = Date.now() - 3 * HOUR
    const to = Date.now()
    await router.push(`/failures?from=${from}&to=${to}`)
    await flushPromises()

    const params = latestListRequest()
    expect(params.get('from')).toBe(String(from))
    expect(params.get('to')).toBe(String(to))
  })

  it('renders the list empty state in Chinese', async () => {
    stubFetch({ rows: [], unlabeled: 0 })
    const { wrapper } = await mountView('/failures?windowMinutes=10080')

    expect(wrapper.text()).toContain('当前区间没有失败记录。')
    expect(wrapper.text()).not.toContain('No failures in this window.')
  })
})
