import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ThinkingBlock from '@/components/ThinkingBlock.vue'
import type { ThinkingSegment } from '@/lib/chatModel'

function segment(text = '先看 KPI。'): ThinkingSegment {
  return { kind: 'thinking', messageId: 'r1', text }
}

function openState(wrapper: ReturnType<typeof mount>) {
  return wrapper.get('details').element.open
}

describe('ThinkingBlock', () => {
  it('is collapsed once the run is over', () => {
    const wrapper = mount(ThinkingBlock, { props: { segment: segment(), running: false } })

    expect(openState(wrapper)).toBe(false)
    expect(wrapper.text()).toContain('先看 KPI。')
  })

  it('follows the stream while the run is still going', async () => {
    const wrapper = mount(ThinkingBlock, { props: { segment: segment(), running: true } })
    expect(openState(wrapper)).toBe(true)

    await wrapper.setProps({ running: false })
    expect(openState(wrapper)).toBe(false)
  })

  it('keeps a reading choice the user made', async () => {
    const wrapper = mount(ThinkingBlock, { props: { segment: segment(), running: true } })

    await wrapper.get('summary').trigger('click')
    expect(openState(wrapper)).toBe(false)

    await wrapper.setProps({ running: false })
    expect(openState(wrapper)).toBe(false)
  })

  it('lets the user reopen a thought that was auto-collapsed', async () => {
    const wrapper = mount(ThinkingBlock, { props: { segment: segment(), running: true } })
    await wrapper.setProps({ running: false })
    expect(openState(wrapper)).toBe(false)

    await wrapper.get('summary').trigger('click')
    expect(openState(wrapper)).toBe(true)
  })
})
