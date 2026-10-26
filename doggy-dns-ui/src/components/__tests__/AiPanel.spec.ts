import { mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { describe, expect, it } from 'vitest'

import AiPanel from '@/components/AiPanel.vue'

function mountPanel(props: { telemetryDown: boolean; rawQueryDown?: boolean }) {
  return mount(AiPanel, {
    props,
    global: { plugins: [createPinia()] },
  })
}

describe('AiPanel', () => {
  it('disables the composer when the telemetry source is down', () => {
    const wrapper = mountPanel({ telemetryDown: true })
    expect(wrapper.find('textarea').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('OpenObserve 不可用')
  })

  it('keeps the composer enabled when only raw query is down', () => {
    const wrapper = mountPanel({ telemetryDown: false, rawQueryDown: true })
    expect(wrapper.find('textarea').attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).toContain('原始查询暂不可用')
  })
})
