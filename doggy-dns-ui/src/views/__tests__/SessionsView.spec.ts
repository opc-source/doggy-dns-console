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

function mountView() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.endsWith('/sessions')) {
        return new Response(JSON.stringify([session]), { status: 200 })
      }
      return new Response(JSON.stringify(session), { status: 200 })
    }),
  )
  const router = createRouter({ history: createMemoryHistory(), routes })
  const wrapper = mount(SessionsView, { global: { plugins: [createPinia(), router] } })
  return wrapper
}

describe('SessionsView', () => {
  it('replays a transcript with thinking, tool calls and answers in order', async () => {
    const wrapper = mountView()
    await flushPromises()
    await flushPromises()

    expect(wrapper.text()).toContain('你好')
    expect(wrapper.findAll('[data-segment]').map((el) => el.attributes('data-segment'))).toEqual([
      'text',
      'thinking',
      'tool',
      'text',
    ])
    expect(wrapper.get('[data-test="tool-args"]').text()).toBe('{\n  "windowMinutes": 60\n}')
  })
})
