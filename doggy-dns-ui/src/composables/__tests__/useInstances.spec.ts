import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Instance } from '@/api/types'
import { useInstanceDetail } from '@/composables/useInstances'
import { emptyDraft, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000
const DAY = 24 * HOUR

const host: Instance = {
  id: 'n1',
  host: 'dns-node-1',
  version: '0.1.0',
  online: true,
  lastSeen: 1,
  mcpReachable: true,
}

function detailBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    instance: host,
    pid: 4242,
    status: 'running',
    uptimeSeconds: 3600,
    listenAddr: '0.0.0.0:15353',
    protocols: ['udp'],
    plugins: [],
    configToml: '',
    redactedKeys: [],
    runtime: { signals: {} },
    failureCount: 0,
    ...overrides,
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

const wrappers: VueWrapper[] = []
let fetches: string[] = []

// Pinned clock, exactly like `useOverview.spec.ts`: absolute brackets and the
// future-tolerance rule in `validateBracket` must not drift while a case runs.
const NOW = 1_761_436_800_000

// Mount the detail composable through a host component so `watch` runs inside a
// reactive effect scope the wrapper's unmount can retire, the same way
// `useOverview.spec.ts` mounts its host. The case owns the `id`/`draft` refs, so
// it can close the drawer or swap the bracket and have the composable react.
function mountDetail(
  id: Ref<string | null>,
  draft: Ref<FilterDraft>,
  respond: (callIndex: number) => Response | Promise<Response> = () => json(detailBody()),
) {
  fetches = []
  let calls = 0
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      fetches.push(String(input))
      return respond(calls++)
    }),
  )
  let detail!: ReturnType<typeof useInstanceDetail>
  const wrapper = mount(
    defineComponent({
      setup() {
        detail = useInstanceDetail(id, draft)
        return () => h('div')
      },
    }),
  )
  wrappers.push(wrapper)
  return { detail, wrapper }
}

function detailFetches() {
  return fetches.filter((url) => url.includes('/api/instances/'))
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
})
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('useInstanceDetail refusal', () => {
  // Item 4: a hand-typed `/instances?from=<ms>` with no `to` used to send NO
  // range at all (the old code called `toRequestQuery`, which drops a lone bound)
  // and silently read the server's default hour while the URL named a different
  // bracket. The dashboard refuses the same case locally; the drawer must too.
  it('refuses a lone from without asking the server', async () => {
    const draft = ref<FilterDraft>({ ...emptyDraft(), fromMs: NOW - HOUR })
    const { detail } = mountDetail(ref('n1'), draft)
    await flushPromises()

    expect(detailFetches()).toHaveLength(0)
    expect(detail.invalid.value).toContain('起止')
    expect(detail.detail.value).toBeNull()
  })

  it('refuses a lone to without asking the server', async () => {
    const draft = ref<FilterDraft>({ ...emptyDraft(), toMs: NOW })
    const { detail } = mountDetail(ref('n1'), draft)
    await flushPromises()

    expect(detailFetches()).toHaveLength(0)
    expect(detail.invalid.value).toContain('起止')
    expect(detail.detail.value).toBeNull()
  })

  it('clears the refusal once the URL carries a legal bracket', async () => {
    const draft = ref<FilterDraft>({ ...emptyDraft(), fromMs: NOW - HOUR })
    const { detail } = mountDetail(ref('n1'), draft)
    await flushPromises()
    expect(detail.invalid.value).toContain('起止')

    draft.value = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW }
    await flushPromises()
    expect(detail.invalid.value).toBeNull()
    expect(detail.detail.value).not.toBeNull()
    expect(detailFetches()).toHaveLength(1)
  })

  it('keeps a legal relative window flowing', async () => {
    const draft = ref<FilterDraft>({ ...emptyDraft(), windowMinutes: 60 })
    const { detail } = mountDetail(ref('n1'), draft)
    await flushPromises()

    expect(detail.invalid.value).toBeNull()
    expect(detailFetches()).toHaveLength(1)
    expect(detailFetches()[0]).toContain('windowMinutes=60')
  })
})

describe('useInstanceDetail ordering', () => {
  // Item 5: the detail fetch had no `++seq` guard. A slow seven-day `COUNT(*)`
  // can land after a fresh one-hour response and overwrite the drawer's number.
  // The discriminating completion order is the NEWEST (hour) resolving first and
  // the superseded week resolving LATE — resolving in fire order would land last
  // and win even with no guard, proving nothing.
  it('renders the second answer when the older week lands after the fresh hour', async () => {
    const week = deferred<Response>()
    const hour = deferred<Response>()
    const draft = ref<FilterDraft>({ ...emptyDraft(), fromMs: NOW - 7 * DAY, toMs: NOW })
    const { detail } = mountDetail(ref('n1'), draft, (index) =>
      index === 0 ? week.promise : hour.promise,
    )
    await flushPromises()
    expect(detailFetches()).toHaveLength(1)

    // The operator narrows to the hour while the week's COUNT is still running.
    draft.value = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW }
    await flushPromises()
    expect(detailFetches()).toHaveLength(2)

    // The fast hour lands first, then the slow week lands late.
    hour.resolve(json(detailBody({ failureCount: 7 })))
    await flushPromises()
    expect(detail.detail.value?.failureCount).toBe(7)

    week.resolve(json(detailBody({ failureCount: 111 })))
    await flushPromises()
    // Without the guard the late week would overwrite the fresh hour.
    expect(detail.detail.value?.failureCount).toBe(7)
  })

  it('clears a stale detail error when the drawer closes', async () => {
    const id = ref<string | null>('n1')
    const draft = ref<FilterDraft>({ ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW })
    const { detail } = mountDetail(id, draft, () =>
      json({ error: { code: 'boom', message: 'detail down' } }, 500),
    )
    await flushPromises()
    expect(detail.error.value).toContain('detail down')

    // Closing the drawer cleared `detail` today but left `error` set, so the stale
    // failure survived into the next open. It must clear on close.
    id.value = null
    await flushPromises()
    expect(detail.detail.value).toBeNull()
    expect(detail.error.value).toBeNull()
  })
})
