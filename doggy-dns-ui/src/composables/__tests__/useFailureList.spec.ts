import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { AnalysisMarker, FailureGroup } from '@/api/types'
import { useFailureList } from '@/composables/useFailureList'
import { emptyDraft, relativeDraft, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000

function failure(key: string, traceId: string | null, query = 'a.example.'): FailureGroup {
  return {
    key,
    traceId,
    query,
    qtype: 'A',
    instance: traceId === null ? null : 'dns-1',
    authority: 'forward',
    responseCode: 'SERVFAIL',
    error: 'timeout',
    count: 2,
    firstSeen: 1,
    lastSeen: 2,
    durationMs: traceId === null ? null : 10,
  }
}

// t1 appears twice (duplicate id), t2 once and one group has no trace id.
const failures: FailureGroup[] = [
  failure('trace:t1', 't1'),
  failure('trace:t1-alt', 't1'),
  failure('trace:t2', 't2'),
  failure('query:b.example.', null, 'b.example.'),
]

function markerFor(traceId: string, id: string): AnalysisMarker {
  return {
    id,
    traceId,
    status: 'succeeded',
    verdictCategory: 'upstream_timeout',
    confidence: 'high',
    attempt: 1,
    createdAt: 1,
    finishedAt: 2,
  }
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

interface Call {
  url: string
  method: string
}

function stubFetch(
  markers: () => Response | Promise<Response>,
  failGroups: () => boolean = () => false,
  uncovered: () => Response | Promise<Response> = () => json({ unlabeled: 1 }),
  groups: () => Response | Promise<Response> = () => json(failures),
): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      calls.push({ url, method: init?.method ?? 'GET' })
      if (url.includes('/api/analysis/markers')) return markers()
      // `/api/failures/uncovered` is a prefix of `/api/failures`, so it must be
      // matched first.
      if (url.includes('/api/failures/uncovered')) return uncovered()
      if (failGroups()) {
        return json({ error: { code: 'boom', message: 'failures down' } }, 500)
      }
      return groups()
    }),
  )
  return calls
}

type List = ReturnType<typeof useFailureList>

interface MountedList {
  list: List
  fetches: Call[]
  groups: List['groups']
  uncovered: List['uncovered']
  error: List['error']
  timerSet: boolean
  draft: Ref<FilterDraft>
  wrapper: VueWrapper
}

const wrappers: VueWrapper[] = []

function mountList(
  draft: Ref<FilterDraft> = ref(relativeDraft(60)),
  responses: {
    markers?: () => Response | Promise<Response>
    failGroups?: () => boolean
    uncovered?: () => Response | Promise<Response>
    groups?: () => Response | Promise<Response>
  } = {},
): MountedList {
  const fetches = stubFetch(
    responses.markers ?? (() => json([markerFor('t1', 'm1'), markerFor('t2', 'm2')])),
    responses.failGroups ?? (() => false),
    responses.uncovered ?? (() => json({ unlabeled: 1 })),
    responses.groups ?? (() => json(failures)),
  )
  const interval = vi.spyOn(globalThis, 'setInterval')
  let list!: List
  const Host = defineComponent({
    setup() {
      list = useFailureList(draft)
      return () => h('div')
    },
  })
  // mount() runs onMounted synchronously, so the spy count read right after
  // mount returns answers whether restartTimer() scheduled the 15 s poll.
  const wrapper = mount(Host)
  wrappers.push(wrapper)
  return {
    list,
    fetches,
    groups: list.groups,
    uncovered: list.uncovered,
    error: list.error,
    timerSet: interval.mock.calls.length > 0,
    draft,
    wrapper,
  }
}

afterEach(() => {
  // Unmounting clears the 15 s refresh interval.
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('useFailureList markers', () => {
  it('requests exactly the unique non-null trace ids and drops unrequested marks', async () => {
    const { fetches, list } = mountList(ref(relativeDraft(60)), {
      markers: () => json([markerFor('t1', 'm1'), markerFor('t2', 'm2'), markerFor('t9', 'm9')]),
    })

    await flushPromises()

    const markerCall = fetches.find((call) => call.url.includes('/api/analysis/markers'))
    expect(markerCall).toBeDefined()
    const url = new URL(markerCall!.url, 'http://x')
    expect(url.pathname).toBe('/api/analysis/markers')
    expect(url.searchParams.get('traceIds')).toBe('t1,t2')
    expect(markerCall!.url).not.toContain('null')
    expect(new Set(list.markers.value)).toEqual(new Set(['t1', 't2']))
    expect(fetches.every((call) => call.method !== 'POST')).toBe(true)
  })

  it('keeps groups rendered and the error clear when markers fail', async () => {
    const { list } = mountList(ref(relativeDraft(60)), {
      markers: () => json({ error: { code: 'boom', message: 'markers down' } }, 500),
    })

    await flushPromises()

    expect(list.groups.value).toHaveLength(4)
    expect(list.error.value).toBeNull()
    expect(list.markers.value.size).toBe(0)
  })

  it('clears the markers when a group refresh fails', async () => {
    let failGroups = false
    const { draft, list } = mountList(ref(relativeDraft(60)), { failGroups: () => failGroups })

    await flushPromises()
    expect(new Set(list.markers.value)).toEqual(new Set(['t1', 't2']))
    expect(list.error.value).toBeNull()

    failGroups = true
    draft.value = relativeDraft(120)
    await flushPromises()

    expect(list.error.value).not.toBeNull()
    expect(list.markers.value.size).toBe(0)
  })
})

describe('useFailureList filter drafts', () => {
  it('sends windowMinutes for a relative draft', async () => {
    const { fetches, wrapper } = mountList(ref(relativeDraft(120)))
    await flushPromises()

    expect(fetches[0]!.url).toContain('windowMinutes=120')
    expect(fetches[0]!.url).not.toContain('from=')
    wrapper.unmount()
  })

  it('sends from/to and stops polling for an absolute draft', async () => {
    vi.useFakeTimers()
    const now = Date.now()
    const draft = ref({
      ...emptyDraft(),
      fromMs: now - 3 * HOUR,
      toMs: now,
      windowMinutes: null,
    })
    const { fetches, timerSet, wrapper } = mountList(draft)
    await flushPromises()

    expect(fetches[0]!.url).toContain(`from=${now - 3 * HOUR}`)
    expect(fetches[0]!.url).toContain(`to=${now}`)
    expect(fetches[0]!.url).not.toContain('windowMinutes=')
    expect(timerSet).toBe(false)

    const initialFetchCount = fetches.length
    vi.advanceTimersByTime(60_000)
    await flushPromises()
    expect(fetches.length).toBe(initialFetchCount)
    wrapper.unmount()
    vi.useRealTimers()
  })

  it('blocks the request and reports the invalid bracket', async () => {
    const draft = ref({ ...emptyDraft(), fromMs: 10, toMs: 20, windowMinutes: null })
    const { fetches, error, wrapper } = mountList(draft)
    await flushPromises()

    expect(fetches).toHaveLength(0)
    expect(error.value).toMatch(/5 分钟/)
    wrapper.unmount()
  })

  it('refetches when the draft changes and clears the old badges', async () => {
    const draft = ref(relativeDraft(60))
    // The first bracket answers with two badges; the re-requested second
    // bracket answers empty, so surviving badges would be the old ones.
    let markerCalls = 0
    const { fetches, groups, list, wrapper } = mountList(draft, {
      markers: () =>
        markerCalls++ === 0
          ? json([markerFor('t1', 'm1'), markerFor('t2', 'm2')])
          : json([]),
    })
    await flushPromises()
    expect(groups.value.length).toBeGreaterThan(0)
    expect(new Set(list.markers.value)).toEqual(new Set(['t1', 't2']))

    draft.value = relativeDraft(1440)
    await flushPromises()

    const listCalls = fetches.filter((entry) => entry.url.startsWith('/api/failures?'))
    expect(listCalls[listCalls.length - 1]!.url).toContain('windowMinutes=1440')
    const markerRequests = fetches.filter((entry) =>
      entry.url.startsWith('/api/analysis/markers'),
    )
    expect(markerRequests).toHaveLength(2)
    expect(list.markers.value.size).toBe(0)
    wrapper.unmount()
  })

  it('asks for the uncovered count only when an instance is selected', async () => {
    const draft = ref({ ...relativeDraft(60), instance: 'dns-node-2' })
    const { fetches, uncovered, wrapper } = mountList(draft)
    await flushPromises()

    expect(fetches.some((entry) => entry.url.startsWith('/api/failures/uncovered?'))).toBe(true)
    expect(uncovered.value).toBe(1)

    fetches.length = 0
    draft.value = relativeDraft(60)
    await flushPromises()
    expect(fetches.some((entry) => entry.url.startsWith('/api/failures/uncovered?'))).toBe(false)
    expect(uncovered.value).toBe(0)
    wrapper.unmount()
  })

  it('drops an uncovered answer from a draft the operator already replaced', async () => {
    let releaseStale!: (response: Response) => void
    const stale = new Promise<Response>((resolve) => {
      releaseStale = resolve
    })
    let uncoveredCalls = 0
    const draft = ref({ ...relativeDraft(60), instance: 'dns-node-1' })
    const { fetches, uncovered, wrapper } = mountList(draft, {
      uncovered: () => {
        uncoveredCalls += 1
        return uncoveredCalls === 1 ? stale : json({ unlabeled: 1 })
      },
    })

    await flushPromises()
    expect(
      fetches.filter((entry) => entry.url.startsWith('/api/failures/uncovered?')),
    ).toHaveLength(1)
    expect(uncovered.value).toBe(0)

    draft.value = { ...relativeDraft(1440), instance: 'dns-node-2' }
    await flushPromises()
    expect(uncovered.value).toBe(1)

    // The old draft's count lands after the new one owns the notice; a late
    // answer must not overwrite it, for an absolute bracket never re-polls.
    releaseStale(json({ unlabeled: 7 }))
    await flushPromises()
    expect(uncovered.value).toBe(1)
    wrapper.unmount()
  })

  it('drops an uncovered answer that lands after the bracket was refused', async () => {
    let releaseStale!: (response: Response) => void
    const stale = new Promise<Response>((resolve) => {
      releaseStale = resolve
    })
    let uncoveredCalls = 0
    const draft = ref({ ...relativeDraft(60), instance: 'dns-node-1' })
    const { uncovered, wrapper } = mountList(draft, {
      uncovered: () => {
        uncoveredCalls += 1
        return uncoveredCalls === 1 ? stale : json({ unlabeled: 3 })
      },
    })

    await flushPromises()
    expect(uncoveredCalls).toBe(1)
    expect(uncovered.value).toBe(0)

    // An unusable bracket clears the table and returns before the notice is
    // re-requested, so the count in flight must be retired with it.
    draft.value = { ...draft.value, fromMs: 10, toMs: 20, windowMinutes: null }
    await flushPromises()
    expect(uncoveredCalls).toBe(1)
    expect(uncovered.value).toBe(0)

    releaseStale(json({ unlabeled: 7 }))
    await flushPromises()
    expect(uncovered.value).toBe(0)
    wrapper.unmount()
  })

  it('drops an uncovered answer that lands after the list request failed', async () => {
    let releaseStale!: (response: Response) => void
    const stale = new Promise<Response>((resolve) => {
      releaseStale = resolve
    })
    let uncoveredCalls = 0
    let failGroups = false
    const draft = ref({ ...relativeDraft(60), instance: 'dns-node-1' })
    const { uncovered, wrapper } = mountList(draft, {
      failGroups: () => failGroups,
      uncovered: () => {
        uncoveredCalls += 1
        return uncoveredCalls === 1 ? stale : json({ unlabeled: 3 })
      },
    })

    await flushPromises()
    expect(uncovered.value).toBe(0)

    // The same host filter, now on a bracket whose list call fails: the table
    // is empty, so a late count would be the only thing left on screen.
    failGroups = true
    draft.value = { ...relativeDraft(120), instance: 'dns-node-1' }
    await flushPromises()
    expect(uncoveredCalls).toBe(1)
    expect(uncovered.value).toBe(0)

    releaseStale(json({ unlabeled: 7 }))
    await flushPromises()
    expect(uncovered.value).toBe(0)
    wrapper.unmount()
  })

  it('drops a list answer for a draft the operator already replaced', async () => {
    const first = deferred<Response>()
    const second = deferred<Response>()
    let listCalls = 0
    const { groups, draft } = mountList(ref(relativeDraft(60)), {
      groups: () => {
        listCalls += 1
        return listCalls === 1 ? first.promise : second.promise
      },
    })

    draft.value = relativeDraft(1440)
    await flushPromises() // the watch callback starts the second list request

    second.resolve(json([failure('trace:new', 'new')]))
    await flushPromises()
    first.resolve(json([failure('trace:old', 'old')]))
    await flushPromises()

    expect(groups.value.map((group) => group.key)).toEqual(['trace:new'])
  })

  it('refuses exactly one absolute bound without querying the list', async () => {
    const { error, groups, fetches } = mountList(
      ref({ ...emptyDraft(), fromMs: Date.now() - HOUR, instance: 'dns-1' }),
    )
    await flushPromises()

    expect(error.value).toMatch(/起止/)
    expect(groups.value).toEqual([])
    expect(fetches.some((call) => call.url.startsWith('/api/failures?'))).toBe(false)
    expect(fetches.some((call) => call.url.includes('/uncovered'))).toBe(false)
  })
})
