import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'

import AiPanel from '@/components/AiPanel.vue'
import { routes } from '@/router'
import { useChatStore } from '@/stores/chat'
import { useUiStore } from '@/stores/ui'

function sse(value: unknown) {
  return `data: ${JSON.stringify(value)}\n\n`
}

/**
 * The panel mounted against a stream the test feeds by hand, with a chat list
 * whose scroll geometry the test controls.
 */
function streamingPanel() {
  const pinia = createPinia()
  setActivePinia(pinia)
  const chat = useChatStore()
  let push!: (chunk: string) => void
  let close!: () => void
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      push = (chunk: string) => controller.enqueue(new TextEncoder().encode(chunk))
      close = () => controller.close()
    },
  })
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) =>
      path.endsWith('/sessions')
        ? new Response(JSON.stringify({ id: 's1' }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          })
        : new Response(body, {
            status: 200,
            headers: { 'content-type': 'text/event-stream' },
          }),
    ),
  )

  const wrapper = mount(AiPanel, {
    props: { telemetryDown: false },
    global: { plugins: [createRouter({ history: createMemoryHistory(), routes }), pinia] },
  })
  const list = wrapper.get('[data-testid="chat-list"]').element
  const geometry = { scrollHeight: 400 }
  Object.defineProperty(list, 'scrollHeight', { configurable: true, get: () => geometry.scrollHeight })
  return { wrapper, chat, list, push, close, geometry }
}

function mountPanel(props: { telemetryDown: boolean; rawQueryDown?: boolean }) {
  const router = createRouter({ history: createMemoryHistory(), routes })
  const push = vi.spyOn(router, 'push')
  const pinia = createPinia()
  setActivePinia(pinia)
  const wrapper = mount(AiPanel, {
    props,
    global: { plugins: [pinia, router] },
  })
  return { wrapper, push, ui: useUiStore() }
}

describe('AiPanel', () => {
  it('disables the composer when the telemetry source is down', () => {
    const { wrapper } = mountPanel({ telemetryDown: true })
    expect(wrapper.find('textarea').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('OpenObserve 不可用')
  })

  it('keeps the composer enabled when only raw query is down', () => {
    const { wrapper } = mountPanel({ telemetryDown: false, rawQueryDown: true })
    expect(wrapper.find('textarea').attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).toContain('原始查询暂不可用')
  })

  it('shows the trace a follow-up came from and jumps back to it', async () => {
    const { wrapper, push, ui } = mountPanel({ telemetryDown: false })
    ui.rememberHandoff({
      traceId: 't1',
      verdict: 'upstream_timeout',
      summary: '上游转发超时导致 SERVFAIL',
      fromMs: 1_761_433_200_000,
      toMs: 1_761_436_800_000,
    })
    await wrapper.vm.$nextTick()

    const bar = wrapper.get('[data-testid="handoff-context"]')
    expect(bar.text()).toContain('t1')
    expect(bar.text()).toContain('upstream_timeout')

    await bar.get('[data-test="reopen-failure"]').trigger('click')
    expect(push).toHaveBeenCalledWith({
      name: 'failure',
      params: { traceId: 't1' },
      query: { from: '1761433200000', to: '1761436800000' },
    })

    await bar.get('[data-test="clear-handoff"]').trigger('click')
    expect(ui.handoff).toBeNull()
    expect(wrapper.find('[data-testid="handoff-context"]').exists()).toBe(false)
  })

  it('follows the answer while its text keeps growing', async () => {
    const { chat, list, push, close, geometry } = streamingPanel()
    const pending = chat.send('继续说')
    await flushPromises()
    push(sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '第一段。' }))
    await flushPromises()
    expect(list.scrollTop).toBe(400)

    // Same segment, so a watcher on the segment count alone would miss this growth.
    geometry.scrollHeight = 800
    push(sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '第二段还在写。' }))
    await flushPromises()
    expect(list.scrollTop).toBe(800)

    push(sse({ type: 'RUN_FINISHED' }))
    close()
    await pending
  })

  it('stops following for a reader who scrolled up, and resumes at the bottom', async () => {
    const { chat, list, push, close, geometry } = streamingPanel()
    const pending = chat.send('继续说')
    await flushPromises()

    list.scrollTop = 100
    list.dispatchEvent(new Event('scroll'))
    geometry.scrollHeight = 900
    push(sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '往回看时又来了一个字。' }))
    await flushPromises()
    expect(list.scrollTop).toBe(100)

    list.scrollTop = 860
    list.dispatchEvent(new Event('scroll'))
    geometry.scrollHeight = 1200
    push(sse({ type: 'TEXT_MESSAGE_CONTENT', messageId: 'm1', delta: '回到贴着看。' }))
    await flushPromises()
    expect(list.scrollTop).toBe(1200)

    push(sse({ type: 'RUN_FINISHED' }))
    close()
    await pending
  })

  it('opens with the newest answer in view', async () => {
    const pinia = createPinia()
    setActivePinia(pinia)
    const chat = useChatStore()
    chat.messages.push({
      role: 'assistant',
      segments: [{ kind: 'text', messageId: 'm1', text: '上一轮写完的长回答。' }],
    })

    const wrapper = mount(AiPanel, {
      props: { telemetryDown: false },
      global: {
        plugins: [pinia, createRouter({ history: createMemoryHistory(), routes })],
      },
    })
    // Reopening the panel unmounts and remounts it, so this is a fresh instance
    // over a transcript that already stopped growing.
    const list = wrapper.get('[data-testid="chat-list"]').element
    Object.defineProperty(list, 'scrollHeight', { configurable: true, get: () => 1200 })

    await flushPromises()

    expect(list.scrollTop).toBe(1200)
  })

  it('hides itself and goes fullscreen from its own header', async () => {
    const { wrapper, ui } = mountPanel({ telemetryDown: false })

    await wrapper.get('[data-test="ai-panel-fullscreen"]').trigger('click')
    expect(ui.aiPanelMode).toBe('fullscreen')

    await wrapper.get('[data-test="ai-panel-hide"]').trigger('click')
    expect(ui.aiPanelMode).toBe('hidden')
  })
})
