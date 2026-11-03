import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, describe, expect, it, vi } from 'vitest'

import ConfigPanel from '@/components/ConfigPanel.vue'
import { routes } from '@/router'
import InstancesView from '@/views/InstancesView.vue'

const listed = [
  { id: 'n1', host: 'dns-node-1', version: '0.1.0', online: true, lastSeen: 1, mcpReachable: true },
]

function detailBody(overrides: Record<string, unknown> = {}) {
  return {
    instance: listed[0]!,
    pid: 4242,
    status: 'running',
    uptimeSeconds: 3600,
    listenAddr: '0.0.0.0:15353',
    protocols: ['udp'],
    plugins: [],
    configToml: '',
    redactedKeys: [],
    runtime: { signals: {} },
    // 旧字段 `recentFailures` 到这一步已经不存在:Task 4 把 `InstanceDetail`
    // 换成 `failureCount: Option<u64>`,序列化就是 `failureCount: number | null`。
    failureCount: 0,
    ...overrides,
  }
}

const wrappers: VueWrapper[] = []
let fetches: string[] = []

// The view reads the bracket from `useRoute()` now, so every mount here carries
// the app's own route table in a memory router (same shape as
// `OverviewView.spec.ts`); a hand-copied route table would keep passing after a
// rename. Opening the drawer is local `selected` state — one click on the card,
// not a second route.
async function mountView(
  path: string,
  detail: () => unknown = () => detailBody(),
  list: () => unknown = () => listed,
): Promise<{ router: Router; wrapper: VueWrapper }> {
  fetches = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      fetches.push(url)
      const body = url.includes('/api/instances/') ? detail() : list()
      return new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }),
  )
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(InstancesView, { global: { plugins: [router, createPinia()] } })
  wrappers.push(wrapper)
  await flushPromises()
  await wrapper.find('button.panel').trigger('click')
  await flushPromises()
  return { router, wrapper }
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
})

describe('InstancesView range', () => {
  it('asks the detail for the bracket the URL carries', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=10080')
    const detail = fetches.find((url) => url.includes('/api/instances/'))!
    expect(detail).toContain('windowMinutes=10080')
    wrapper.unmount()
  })

  it('re-requests the detail when the operator picks another window', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=60')
    const before = fetches.filter((url) => url.includes('/api/instances/')).length

    await wrapper.get('[data-testid="quick-7d"]').trigger('click')
    await flushPromises()

    const list = fetches.filter((url) => url.includes('/api/instances/'))
    expect(list.length).toBe(before + 1)
    // Indexed access rather than `.at(-1)`: `tsconfig.vitest.json` clears `lib`,
    // so ES2022 array methods do not type-check in specs.
    expect(list[list.length - 1]).toContain('windowMinutes=10080')
    wrapper.unmount()
  })

  it('sends no time parameter to the list itself', async () => {
    // `/api/instances` is the one endpoint with no bracket: its rows are
    // identity and reachability. A `?windowMinutes=` here would be a lie the
    // server ignores — the hint line is what says so out loud.
    const { wrapper } = await mountView('/instances?windowMinutes=10080')
    expect(fetches.filter((url) => url === '/api/instances')).toHaveLength(1)
    wrapper.unmount()
  })

  it('renders an em dash instead of a fabricated zero', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=60', () =>
      detailBody({ failureCount: null }),
    )
    expect(wrapper.get('[data-testid="failure-count"]').text()).toBe('—')
    wrapper.unmount()
  })

  it('renders a counted zero as a zero', async () => {
    // `??`, not `||`: `0` is an answer ("this node was healthy that week"),
    // `null` is the absence of one. A reviewer-proof pin for that difference.
    const { wrapper } = await mountView('/instances?windowMinutes=60')
    expect(wrapper.get('[data-testid="failure-count"]').text()).toBe('0')
    wrapper.unmount()
  })
})

describe('InstancesView', () => {
  it('shows mcp unreachable without hiding the instance', async () => {
    const offline = [
      { id: 'n2', host: 'n2', version: '0.1.0', online: false, lastSeen: 1, mcpReachable: false },
    ]
    const { wrapper } = await mountView('/instances', () => detailBody({ instance: offline[0]! }), () => offline)
    expect(wrapper.text()).toContain('n2')
    expect(wrapper.text()).toContain('MCP unreachable')
    wrapper.unmount()
  })

  it('renders live runtime signals and redacted keys for an instance', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=60', () =>
      detailBody({
        protocols: ['udp', 'tcp'],
        configToml: '[server]\nport = 15353',
        redactedKeys: ['observability.basic_auth.password'],
        runtime: {
          signals: {
            'nacos.nacos.local': { cachedServices: 42, lastPushSecondsAgo: 3 },
            xdp: { attached: true },
          },
        },
      }),
    )
    expect(wrapper.text()).toContain('cached services 42')
    expect(wrapper.text()).toContain('last push 3s ago')
    expect(wrapper.text()).toContain('fast path attached')
    expect(wrapper.text()).toContain('observability.basic_auth.password')
    wrapper.unmount()
  })

  it('renders config as read-only pre', () => {
    const wrapper = mount(ConfigPanel, { props: { toml: '[server]\nport = 15353' } })
    expect(wrapper.find('pre').text()).toContain('port = 15353')
    expect(wrapper.find('textarea').exists()).toBe(false)
  })
})
