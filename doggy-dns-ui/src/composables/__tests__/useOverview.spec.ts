import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useOverview } from '@/composables/useOverview'
import { emptyDraft, relativeDraft, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000
const DAY = 24 * HOUR
const NOW = 1_761_436_800_000

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

const overviewBody = {
  windowMinutes: 60,
  instancesOnline: 3,
  instancesTotal: 4,
  queries: 10,
  failures: 1,
  successRate: 0.9,
  p95LatencyMs: 12.4,
  series: [],
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

const wrappers: VueWrapper[] = []
let fetches: string[] = []

// 照 `useFailureList.spec.ts` 的 `mountList`:helper 里不 await,`mount()` 已同步
// 跑完 `onMounted`,请求落地由用例自己 `await flushPromises()`。
// 必须用宿主组件挂载:`useOverview` 的首次请求与轮询都登记在 `onMounted`,
// 在 setup 上下文之外直接调用它时 `onMounted` 是空操作,那样"停轮询"会因
// 为什么都没启动而假通过。
function mountOverview(
  draft: Ref<FilterDraft>,
  respond: () => Response | Promise<Response> = () => json(overviewBody),
) {
  fetches = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      fetches.push(String(input))
      return respond()
    }),
  )
  let overview!: ReturnType<typeof useOverview>
  const wrapper = mount(
    defineComponent({
      setup() {
        overview = useOverview(draft)
        return () => h('div')
      },
    }),
  )
  wrappers.push(wrapper)
  return { overview, wrapper }
}

beforeEach(() => {
  vi.useFakeTimers()
  // 钉住时钟:`setFrom` 之类的 `Date.now()` 与 15 s 轮询都要落在可预测的刻度上。
  vi.setSystemTime(NOW)
})
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('useOverview', () => {
  it('asks for the absolute bracket and stops polling it', async () => {
    // D8: a week refreshed every 15 s is a load test, not a live view.
    const draft = ref({ ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW })
    const { wrapper } = mountOverview(draft)
    await flushPromises()

    expect(fetches.length).toBe(1)
    expect(fetches[0]).toContain(`from=${NOW - 3 * HOUR}`)
    expect(fetches[0]).toContain(`to=${NOW}`)
    expect(fetches[0]).not.toContain('windowMinutes=')

    const initial = fetches.length
    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(initial)
    wrapper.unmount()
  })

  it('keeps polling a relative window', async () => {
    const { wrapper } = mountOverview(ref(relativeDraft(60)))
    await flushPromises()
    expect(fetches.length).toBe(1)

    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(4) // 15 s × 3
    wrapper.unmount()
  })

  it('refuses a bad bracket without asking the server', async () => {
    const { overview, wrapper } = mountOverview(ref({ ...emptyDraft(), windowMinutes: 20_000 }))
    await flushPromises()

    expect(fetches.length).toBe(0)
    expect(overview.invalid.value).toContain('10080')
    expect(overview.error.value).toBeNull()
    wrapper.unmount()
  })

  it('drops the previous bracket\'s data when the new one is refused', async () => {
    const draft = ref(relativeDraft(60))
    const { overview, wrapper } = mountOverview(draft)
    await flushPromises()
    expect(overview.data.value).not.toBeNull()

    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await flushPromises()
    // A chart that no longer matches the URL is worse than an empty one.
    expect(overview.data.value).toBeNull()
    expect(overview.invalid.value).toContain('10080')
    wrapper.unmount()
  })

  it('retires an in-flight answer when the bracket that asked for it is refused', async () => {
    // `refresh()` bumps the sequence before it looks at the draft, so the
    // answer still in flight belongs to a bracket that no longer exists.
    // Without that, the late response would re-paint a chart the URL refuses.
    let release!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => {
      release = resolve
    })
    const draft = ref(relativeDraft(60))
    const { overview, wrapper } = mountOverview(draft, () => pending)
    await flushPromises()

    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await flushPromises()
    expect(overview.data.value).toBeNull()

    release(json(overviewBody))
    await flushPromises()
    expect(overview.data.value).toBeNull()
    wrapper.unmount()
  })

  it('releases loading when a refusal supersedes an in-flight request', async () => {
    // The refresh button is `:disabled="loading"` and a refusal re-enters the
    // same early return, so a stuck flag would park a spinner forever on exactly
    // the absolute URLs where manual refresh is the only affordance D8 leaves:
    // nothing resets it, because an absolute bracket installs no timer and the
    // retired request's `finally` deliberately skips clearing.
    let release!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => {
      release = resolve
    })
    const draft = ref(relativeDraft(60))
    const { overview, wrapper } = mountOverview(draft, () => pending)
    await flushPromises()
    expect(overview.loading.value).toBe(true)

    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await flushPromises()
    expect(overview.invalid.value).toContain('10080')
    expect(overview.loading.value).toBe(false)

    // Honest on both halves: clearing the flag must not re-admit the late answer
    // for the bracket the URL just refused.
    release(json(overviewBody))
    await flushPromises()
    expect(overview.data.value).toBeNull()
    expect(overview.loading.value).toBe(false)
    wrapper.unmount()
  })

  it('tears the poll down when a relative draft becomes an absolute bracket', async () => {
    // D8 is two halves: an absolute bracket asks for nothing new, and switching
    // to one stops the interval the relative draft was living on. Without the
    // teardown a week-old `from`/`to` keeps being re-queried every 15 s.
    const draft = ref(relativeDraft(60))
    const { wrapper } = mountOverview(draft)
    await flushPromises()
    expect(fetches.length).toBe(1)

    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(4) // 15 s × 3, the relative window is polling

    draft.value = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW }
    await flushPromises()
    // The watch's own refresh, not a poll: the absolute bracket is a fixed question.
    expect(fetches.length).toBe(5)

    const frozen = fetches.length
    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(frozen)
    wrapper.unmount()
  })

  it('installs exactly one interval when a bracket round-trips absolute→relative', async () => {
    // Item 7a. The teardown above only pins one direction (relative→absolute).
    // `restartTimer()` clears before it installs, so switching BACK to a relative
    // window must leave a single 15 s interval, not two stacked: a double-install
    // would advance roughly twice the fetches over the same window.
    const draft = ref(relativeDraft(60))
    const { wrapper } = mountOverview(draft)
    await flushPromises()
    expect(fetches.length).toBe(1)

    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(4) // the relative window polls: 15 s × 3

    draft.value = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW }
    await flushPromises()
    expect(fetches.length).toBe(5) // the watch's own refresh, no timer installed

    draft.value = relativeDraft(60)
    await flushPromises()
    expect(fetches.length).toBe(6) // the switch-back refresh

    const beforeReinstall = fetches.length
    await vi.advanceTimersByTimeAsync(45_000)
    // Exactly ONE interval's worth of new fetches (3), not two intervals' worth (6).
    expect(fetches.length).toBe(beforeReinstall + 3)
    wrapper.unmount()
  })

  it('keeps loading true until the newest overlapping request settles', async () => {
    // Item 7b. Two VALID, overlapping brackets in flight at once: the older one
    // resolving must NOT release the spinner while the newest is still pending —
    // otherwise a brief success would flash for a bracket the URL already replaced.
    const older = deferred<Response>()
    const newer = deferred<Response>()
    let calls = 0
    const respond = () => (calls++ === 0 ? older.promise : newer.promise)

    const draft = ref({ ...emptyDraft(), fromMs: NOW - 2 * DAY, toMs: NOW })
    const { overview, wrapper } = mountOverview(draft, respond)
    await flushPromises()
    expect(fetches.length).toBe(1)
    expect(overview.loading.value).toBe(true)

    draft.value = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW }
    await flushPromises()
    expect(fetches.length).toBe(2)
    expect(overview.loading.value).toBe(true)

    // The superseded two-day answer lands first; the newest hour is still running.
    older.resolve(json(overviewBody))
    await flushPromises()
    expect(overview.loading.value).toBe(true)
    expect(overview.data.value).toBeNull()

    // Only the newest response owns the spinner.
    newer.resolve(json({ ...overviewBody, queries: 999 }))
    await flushPromises()
    expect(overview.loading.value).toBe(false)
    expect(overview.data.value?.queries).toBe(999)
    wrapper.unmount()
  })
})
