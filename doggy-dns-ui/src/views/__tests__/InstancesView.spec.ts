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

  it('renders live runtime signals and redacted keys for an instance', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input)
        if (url.includes('/api/instances/n1')) {
          return new Response(
            JSON.stringify({
              instance: {
                id: 'n1',
                host: 'n1',
                version: '0.1.0',
                online: true,
                lastSeen: 1,
                mcpReachable: true,
              },
              pid: 4242,
              status: 'running',
              uptimeSeconds: 3600,
              startedAt: '2026-10-25T09:00:00Z',
              listenAddr: '0.0.0.0:15353',
              protocols: ['udp', 'tcp'],
              plugins: [],
              configToml: '[server]\nport = 15353',
              redactedKeys: ['observability.basic_auth.password'],
              runtime: {
                signals: {
                  'nacos.nacos.local': { cachedServices: 42, lastPushSecondsAgo: 3 },
                  xdp: { attached: true },
                },
              },
              recentFailures: 0,
            }),
            { status: 200 },
          )
        }
        return new Response(
          JSON.stringify([
            {
              id: 'n1',
              host: 'n1',
              version: '0.1.0',
              online: true,
              lastSeen: 1,
              mcpReachable: true,
            },
          ]),
          { status: 200 },
        )
      }),
    )
    const wrapper = mount(InstancesView, { global: { plugins: [createPinia()] } })
    await flushPromises()
    await wrapper.find('button.panel').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('cached services 42')
    expect(wrapper.text()).toContain('last push 3s ago')
    expect(wrapper.text()).toContain('fast path attached')
    expect(wrapper.text()).toContain('observability.basic_auth.password')
  })

  it('renders config as read-only pre', () => {
    const wrapper = mount(ConfigPanel, { props: { toml: '[server]\nport = 15353' } })
    expect(wrapper.find('pre').text()).toContain('port = 15353')
    expect(wrapper.find('textarea').exists()).toBe(false)
  })
})
