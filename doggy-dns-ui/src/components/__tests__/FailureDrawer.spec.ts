import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { describe, expect, it, vi } from 'vitest'

import FailureDrawer from '@/components/FailureDrawer.vue'
import { useChatStore } from '@/stores/chat'

const detail = {
  group: {
    key: 'trace:t1',
    traceId: 't1',
    query: 'a.example.',
    qtype: 'A',
    instance: 'dns-node-1',
    authority: 'forward',
    responseCode: 'SERVFAIL',
    error: 'timeout',
    count: 2,
    firstSeen: 1,
    lastSeen: 2,
    durationMs: 10,
  },
  logs: [
    {
      ts: 1,
      severity: 'ERROR',
      message: 'upstream forward lookup failed',
      traceId: 't1',
      spanId: 's1',
      fields: { query: 'a.example.' },
    },
  ],
  spans: [{ spanId: 's1', name: 'dns.error', durationMs: 10, status: 'ERROR', attributes: {} }],
}

describe('FailureDrawer', () => {
  it('opens, shows trace evidence and closes on escape', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(detail), { status: 200 })))
    const pinia = createPinia()
    setActivePinia(pinia)
    const wrapper = mount(FailureDrawer, {
      props: { traceId: 't1' },
      global: { plugins: [pinia] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('t1')
    expect(wrapper.text()).toContain('upstream forward lookup failed')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('sends the current filter to the ai panel', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(detail), { status: 200 })))
    const pinia = createPinia()
    setActivePinia(pinia)
    const chat = useChatStore()
    const send = vi.spyOn(chat, 'send').mockResolvedValue(undefined)
    const wrapper = mount(FailureDrawer, {
      props: { traceId: 't1' },
      global: { plugins: [pinia] },
    })
    await flushPromises()
    await wrapper.find('[data-test="analyze"]').trigger('click')
    expect(send).toHaveBeenCalledWith(expect.stringContaining('trace_id=t1'))
  })
})
