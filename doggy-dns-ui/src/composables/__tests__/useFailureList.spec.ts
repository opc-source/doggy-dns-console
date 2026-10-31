import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { AnalysisMarker, FailureGroup } from '@/api/types'
import { useFailureList } from '@/composables/useFailureList'

const failures: FailureGroup[] = [
  {
    key: 'trace:t1',
    traceId: 't1',
    query: 'a.example.',
    qtype: 'A',
    instance: 'dns-1',
    authority: 'forward',
    responseCode: 'SERVFAIL',
    error: 'timeout',
    count: 2,
    firstSeen: 1,
    lastSeen: 2,
    durationMs: 10,
  },
  {
    key: 'query:b.example.',
    traceId: null,
    query: 'b.example.',
    qtype: 'A',
    instance: null,
    authority: 'forward',
    responseCode: 'SERVFAIL',
    error: 'timeout',
    count: 1,
    firstSeen: 1,
    lastSeen: 2,
    durationMs: null,
  },
]

const marker: AnalysisMarker = {
  id: 'm1',
  traceId: 't1',
  status: 'succeeded',
  verdictCategory: 'upstream_timeout',
  confidence: 'high',
  attempt: 1,
  createdAt: 1,
  finishedAt: 2,
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

function stubFetch(markers: () => Response | Promise<Response>): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      calls.push({ url, method: init?.method ?? 'GET' })
      if (url.includes('/api/analysis/markers')) return markers()
      return json(failures)
    }),
  )
  return calls
}

type List = ReturnType<typeof useFailureList>

const wrappers: VueWrapper[] = []

function mountList(): List {
  let list!: List
  const Host = defineComponent({
    setup() {
      list = useFailureList(ref(60))
      return () => h('div')
    },
  })
  const wrapper = mount(Host)
  wrappers.push(wrapper)
  return list
}

afterEach(() => {
  // Unmounting clears the 15 s refresh interval.
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('useFailureList markers', () => {
  it('marks the trace ids returned for the listed groups', async () => {
    const calls = stubFetch(() => json([marker]))
    const list = mountList()

    await flushPromises()

    const markerCall = calls.find((call) => call.url.includes('/api/analysis/markers'))
    expect(markerCall?.url).toContain('/api/analysis/markers?traceIds=t1')
    expect(markerCall?.url).not.toContain('null')
    expect(list.markers.value.has('t1')).toBe(true)
    expect(calls.every((call) => call.method !== 'POST')).toBe(true)
  })

  it('keeps groups rendered and the error clear when markers fail', async () => {
    stubFetch(() => json({ error: { code: 'boom', message: 'markers down' } }, 500))
    const list = mountList()

    await flushPromises()

    expect(list.groups.value).toHaveLength(2)
    expect(list.error.value).toBeNull()
    expect(list.markers.value.size).toBe(0)
  })
})
