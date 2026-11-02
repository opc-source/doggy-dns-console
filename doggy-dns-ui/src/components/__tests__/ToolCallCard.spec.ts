import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolCallCard from '@/components/ToolCallCard.vue'
import type { ToolSegment } from '@/lib/chatModel'

function tool(props: Partial<ToolSegment> = {}): ToolSegment {
  return {
    kind: 'tool',
    toolCallId: 'tc_0',
    name: 'get_overview',
    args: '{"windowMinutes":60}',
    result: '{"queries":128934}',
    status: 'done',
    ...props,
  }
}

describe('ToolCallCard', () => {
  it('pretty-prints the JSON arguments and result', () => {
    const wrapper = mount(ToolCallCard, { props: { call: tool() } })

    expect(wrapper.get('[data-test="tool-args"]').text()).toBe('{\n  "windowMinutes": 60\n}')
    expect(wrapper.get('[data-test="tool-result"]').text()).toBe('{\n  "queries": 128934\n}')
  })

  it('keeps a non-JSON payload readable', () => {
    const wrapper = mount(ToolCallCard, { props: { call: tool({ args: 'SELECT 1', result: undefined }) } })

    expect(wrapper.get('[data-test="tool-args"]').text()).toBe('SELECT 1')
    expect(wrapper.find('[data-test="tool-result"]').exists()).toBe(false)
  })

  it('marks the state of the call', () => {
    const wrapper = mount(ToolCallCard, { props: { call: tool({ status: 'error' }) } })

    expect(wrapper.text()).toContain('get_overview')
    expect(wrapper.text()).toContain('error')
  })
})
