import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { AnalysisMarker, FailureGroup } from '@/api/types'
import { useFailureList } from '@/composables/useFailureList'

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

interface Call {
  url: string
  method: string
}

function stubFetch(
  markers: () => Response | Promise<Response>,
  failGroups: () => boolean = () => false,
): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      calls.push({ url, method: init?.method ?? 'GET' })
      if (url.includes('/api/analysis/markers')) return markers()
      if (failGroups()) {
        return json({ error: { code: 'boom', message: 'failures down' } }, 500)
      }
      return json(failures)
    }),
  )
  return calls
}

type List = ReturnType<typeof useFailureList>

const wrappers: VueWrapper[] = []

function mountList(): { list: List; windowMinutes: Ref<number> } {
  const windowMinutes = ref(60)
  let list!: List
  const Host = defineComponent({
    setup() {
      list = useFailureList(windowMinutes)
      return () => h('div')
    },
  })
  const wrapper = mount(Host)
  wrappers.push(wrapper)
  return { list, windowMinutes }
}

afterEach(() => {
  // Unmounting clears the 15 s refresh interval.
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('useFailureList markers', () => {
  it('requests exactly the unique non-null trace ids and drops unrequested marks', async () => {
    const calls = stubFetch(() =>
      json([markerFor('t1', 'm1'), markerFor('t2', 'm2'), markerFor('t9', 'm9')]),
    )
    const { list } = mountList()

    await flushPromises()

    const markerCall = calls.find((call) => call.url.includes('/api/analysis/markers'))
    expect(markerCall).toBeDefined()
    const url = new URL(markerCall!.url, 'http://x')
    expect(url.pathname).toBe('/api/analysis/markers')
    expect(url.searchParams.get('traceIds')).toBe('t1,t2')
    expect(markerCall!.url).not.toContain('null')
    expect(new Set(list.markers.value)).toEqual(new Set(['t1', 't2']))
    expect(calls.every((call) => call.method !== 'POST')).toBe(true)
  })

  it('keeps groups rendered and the error clear when markers fail', async () => {
    stubFetch(() => json({ error: { code: 'boom', message: 'markers down' } }, 500))
    const { list } = mountList()

    await flushPromises()

    expect(list.groups.value).toHaveLength(4)
    expect(list.error.value).toBeNull()
    expect(list.markers.value.size).toBe(0)
  })

  it('clears the markers when a group refresh fails', async () => {
    let failGroups = false
    stubFetch(
      () => json([markerFor('t1', 'm1'), markerFor('t2', 'm2')]),
      () => failGroups,
    )
    const { list, windowMinutes } = mountList()

    await flushPromises()
    expect(new Set(list.markers.value)).toEqual(new Set(['t1', 't2']))
    expect(list.error.value).toBeNull()

    failGroups = true
    windowMinutes.value = 120
    await flushPromises()

    expect(list.error.value).not.toBeNull()
    expect(list.markers.value.size).toBe(0)
  })
})
