import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import MessageBubble from '@/components/MessageBubble.vue'
import type { Segment } from '@/lib/chatModel'

function order(segments: Segment[]) {
  const wrapper = mount(MessageBubble, { props: { message: { role: 'assistant', segments } } })
  return wrapper
    .findAll('[data-segment]')
    .map((element) => element.attributes('data-segment'))
}

describe('MessageBubble', () => {
  it('renders thinking, text and tool segments in the order they arrived', () => {
    expect(
      order([
        { kind: 'thinking', messageId: 'r1', text: '先看 KPI。' },
        { kind: 'tool', toolCallId: 'tc_0', name: 'get_overview', args: '{}', status: 'running' },
        { kind: 'text', messageId: 'm1', text: '成功率 99.7%。' },
        { kind: 'tool', toolCallId: 'tc_1', name: 'get_failures', args: '{}', status: 'done' },
      ]),
    ).toEqual(['thinking', 'tool', 'text', 'tool'])
  })

  it('formats a markdown answer', () => {
    const wrapper = mount(MessageBubble, {
      props: {
        message: {
          role: 'assistant',
          segments: [{ kind: 'text', messageId: 'm1', text: '## 结论\n\n- SERVFAIL 12 次' }],
        },
      },
    })

    expect(wrapper.get('[data-segment="text"]').element.querySelector('h2')?.textContent).toBe('结论')
    expect(wrapper.findAll('li')).toHaveLength(1)
  })

  it('shows a placeholder while the answer has produced nothing yet', () => {
    const wrapper = mount(MessageBubble, {
      props: { message: { role: 'assistant', segments: [] }, running: true },
    })

    expect(wrapper.find('[data-test="typing-indicator"]').exists()).toBe(true)
  })

  it('keeps a user message plain text', () => {
    const wrapper = mount(MessageBubble, {
      props: { message: { role: 'user', segments: [{ kind: 'text', messageId: '', text: '整体健康度如何' }] } },
    })

    expect(wrapper.text()).toBe('整体健康度如何')
    expect(wrapper.find('[data-segment="thinking"]').exists()).toBe(false)
  })
})
