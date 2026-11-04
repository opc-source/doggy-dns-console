import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'

import { routes } from '@/router'
import SessionsView from '@/views/SessionsView.vue'

const session = {
  id: 's1',
  appName: 'doggy-dns-console',
  userId: 'local',
  lastUpdateTime: 1_761_436_800,
  state: {},
  events: [
    { author: 'user', content: { role: 'user', parts: [{ text: '你好' }] } },
    {
      author: 'doggy-dns-console',
      content: { role: 'model', parts: [{ thinking: '先查 KPI。' }] },
    },
    {
      author: 'doggy-dns-console',
      content: { role: 'model', parts: [{ args: { windowMinutes: 60 }, id: 'tc_0', name: 'get_overview' }] },
    },
    {
      author: 'doggy-dns-console',
      content: {
        role: 'model',
        parts: [{ functionResponse: { name: 'get_overview', response: { queries: 128934 } }, id: 'tc_0' }],
      },
    },
    { author: 'doggy-dns-console', content: { role: 'model', parts: [{ text: '在。' }] } },
  ],
}

function jsonFetch(payload: unknown) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  })
}

function stubFetch(list: unknown[] = [session], detail: unknown = session) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith('/sessions')) return jsonFetch(list)
      return jsonFetch(detail)
    }),
  )
}

async function mountView(path = '/sessions', list: unknown[] = [session]) {
  stubFetch(list)
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(SessionsView, { global: { plugins: [createPinia(), router] } })
  await flushPromises()
  await flushPromises()
  return { router, wrapper }
}

describe('SessionsView', () => {
  it('replays a transcript with thinking, tool calls and answers in order', async () => {
    const { wrapper } = await mountView()

    expect(wrapper.text()).toContain('你好')
    expect(wrapper.findAll('[data-segment]').map((el) => el.attributes('data-segment'))).toEqual([
      'text',
      'thinking',
      'tool',
      'text',
    ])
    expect(wrapper.get('[data-test="tool-args"]').text()).toBe('{\n  "windowMinutes": 60\n}')
  })

  it('opens the session the URL names', async () => {
    const others = [
      { ...session, id: 's1', state: { title: '第一个' } },
      { ...session, id: 's2', state: { title: '第二个' } },
    ]
    const { wrapper } = await mountView('/sessions/s2', others)

    // The deep link is the selection: the second session is open, not the first.
    expect(wrapper.find('[data-test="selected-session"]').text()).toBe('第二个')
  })

  it('writes the session it auto-selects into the URL', async () => {
    const { router } = await mountView('/sessions')

    // A bare /sessions still shows something, and the address has to say which
    // session that is, or Refresh and Back lose the operator where they started.
    expect(router.currentRoute.value.params.sessionId).toBe('s1')
  })

  it('leaves the address alone when the list arrives off the sessions page', async () => {
    const others = [
      { ...session, id: 's1', state: { title: '第一个' } },
      { ...session, id: 's2', state: { title: '第二个' } },
    ]
    stubFetch(others)
    const router = createRouter({ history: createMemoryHistory(), routes })
    await router.push('/')
    // The window a remount opens: the view is live while the route is not
    // `sessions` yet. Filling in the address then would rewrite the page the
    // operator is standing on.
    mount(SessionsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/')
  })

  it('offers the way back to the failure a handoff session came from', async () => {
    const labelled = {
      ...session,
      state: {
        title: '追问 a.example. · upstream_timeout',
        traceId: 't1',
        analysisId: 'an-9',
        from: 1_761_433_200_000,
        to: 1_761_436_800_000,
      },
    }
    const { wrapper } = await mountView('/sessions/s1', [labelled])

    const jump = wrapper.get('[data-test="failure-jump"]')
    const href = jump.attributes('href') ?? ''
    expect(href).toContain('/failures/t1')
    expect(href).toContain('analysis=an-9')
    // The bracket travels too: without it the drawer would reopen on a window
    // that no longer contains the failure the question was asked about.
    expect(href).toContain('from=1761433200000')
    expect(href).toContain('to=1761436800000')
  })

  it('keeps the jump away from sessions that never came from a failure', async () => {
    const { wrapper } = await mountView()

    expect(wrapper.find('[data-test="failure-jump"]').exists()).toBe(false)
  })
})
