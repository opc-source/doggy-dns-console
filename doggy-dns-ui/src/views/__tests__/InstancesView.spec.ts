import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { describe, expect, it, vi } from 'vitest'

import ConfigPanel from '@/components/ConfigPanel.vue'
import InstancesView from '@/views/InstancesView.vue'

describe('InstancesView', () => {
  it('shows mcp unreachable without hiding the instance', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(
          JSON.stringify([
            { id: 'n2', host: 'n2', version: '0.1.0', online: false, lastSeen: 1, mcpReachable: false },
          ]),
          { status: 200 },
        ),
      ),
    )
    const wrapper = mount(InstancesView, { global: { plugins: [createPinia()] } })
    await flushPromises()
    expect(wrapper.text()).toContain('n2')
    expect(wrapper.text()).toContain('MCP unreachable')
  })

  it('renders config as read-only pre', () => {
    const wrapper = mount(ConfigPanel, { props: { toml: '[server]\nport = 15353' } })
    expect(wrapper.find('pre').text()).toContain('port = 15353')
    expect(wrapper.find('textarea').exists()).toBe(false)
  })
})
