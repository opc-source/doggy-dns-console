import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'

import { routes } from '@/router'
import { mapSessionEvents } from '@/stores/chat'
import SessionsView from '@/views/SessionsView.vue'

describe('SessionsView', () => {
  it('maps adk events to chat messages', () => {
    const events = [
      { author: 'user', content: { role: 'user', parts: [{ text: '你好' }] } },
      { author: 'doggy-dns-console', content: { role: 'model', parts: [{ text: '在。' }] } },
    ]
    expect(mapSessionEvents(events)).toEqual([
      { role: 'user', content: '你好' },
      { role: 'assistant', content: '在。' },
    ])
  })

  it('loads a session transcript', async () => {
    const session = {
      id: 's1',
      appName: 'doggy-dns-console',
      userId: 'local',
      lastUpdateTime: 1_761_436_800,
      state: {},
      events: [
        { author: 'user', content: { role: 'user', parts: [{ text: '你好' }] } },
        { author: 'doggy-dns-console', content: { role: 'model', parts: [{ text: '在。' }] } },
      ],
    }
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input)
        if (url.endsWith('/sessions')) return new Response(JSON.stringify([session]), { status: 200 })
        return new Response(JSON.stringify(session), { status: 200 })
      }),
    )
    const router = createRouter({ history: createMemoryHistory(), routes })
    const wrapper = mount(SessionsView, {
      global: { plugins: [createPinia(), router] },
    })
    await flushPromises()
    await flushPromises()
    expect(wrapper.text()).toContain('你好')
    expect(wrapper.text()).toContain('在。')
  })
})
