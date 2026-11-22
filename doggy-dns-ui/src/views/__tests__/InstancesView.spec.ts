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
    runtime: { signals: {}, logs: { stdout: 'info', otlp: 'info' } },
    controlEnabled: true,
    // 旧字段 `recentFailures` 到这一步已经不存在:Task 4 把 `InstanceDetail`
    // 换成 `failureCount: Option<u64>`,序列化就是 `failureCount: number | null`。
    failureCount: 0,
    ...overrides,
  }
}

const wrappers: VueWrapper[] = []
let fetches: string[] = []
interface Call {
  url: string
  method: string
  body: unknown
}
let calls: Call[] = []

// The view reads the bracket from `useRoute()` now, so every mount here carries
// the app's own route table in a memory router (same shape as
// `OverviewView.spec.ts`); a hand-copied route table would keep passing after a
// rename. Opening the drawer is local `selected` state — one click on the card,
// not a second route.
async function mountView(
  path: string,
  detail: (url?: string) => unknown = () => detailBody(),
  list: () => unknown = () => listed,
  post: () => Response | Promise<Response> = () =>
    new Response(JSON.stringify({ stdout: 'debug', otlp: 'debug' }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
): Promise<{ router: Router; wrapper: VueWrapper }> {
  fetches = []
  calls = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const method = init?.method ?? 'GET'
      let body: unknown = null
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body)
        } catch {
          body = init.body
        }
      }
      calls.push({ url, method, body })
      if (method === 'POST') return post()
      fetches.push(url)
      const payload = url.includes('/api/instances/') ? detail(url) : list()
      return new Response(JSON.stringify(payload), {
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

describe('InstancesView drawer link and refusal', () => {
  // Item 3 (spec D5): the drawer's "view this host's failures" link must carry
  // `instance` PLUS the bracket the number was counted over. Deleting
  // `toRouteQuery(draft)` from that link used to leave the whole suite green
  // while sending the operator to a different window than the one they read.
  it('carries the bracket and the host into the failures link (relative)', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=10080')

    const href = wrapper.get('[data-testid="failures-for-host"]').attributes('href')!
    expect(href).toContain('/failures')
    expect(href).toContain('windowMinutes=10080')
    expect(href).toContain('instance=dns-node-1')
    wrapper.unmount()
  })

  it('carries the absolute bracket and the host into the failures link', async () => {
    const from = 1_761_425_200_000
    const to = 1_761_436_800_000
    const { wrapper } = await mountView(`/instances?from=${from}&to=${to}`)

    const href = wrapper.get('[data-testid="failures-for-host"]').attributes('href')!
    expect(href).toContain(`from=${from}`)
    expect(href).toContain(`to=${to}`)
    expect(href).toContain('instance=dns-node-1')
    // A relative shortcut must not leak alongside an absolute pair.
    expect(href).not.toContain('windowMinutes=')
    wrapper.unmount()
  })

  // Item 4: a hand-typed lone `from` must refuse locally and surface it, exactly
  // the way the dashboard refuses it, rather than silently reading the default hour.
  it('refuses a lone from and asks the detail endpoint nothing', async () => {
    const { wrapper } = await mountView('/instances?from=1761425200000')

    expect(fetches.filter((url) => url.includes('/api/instances/'))).toHaveLength(0)
    expect(wrapper.get('[data-testid="detail-invalid"]').text()).toContain('起止')
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

describe('InstancesView runtime log level', () => {
  it('shows the filters the node reports', async () => {
    const { wrapper } = await mountView('/instances', () =>
      detailBody({
        runtime: { signals: {}, logs: { stdout: 'doggy_dns=debug', otlp: null } },
      }),
    )

    expect(wrapper.get('[data-testid="log-level-stdout"]').text()).toBe('doggy_dns=debug')
    expect(wrapper.get('[data-testid="log-level-otlp"]').text()).toBe('off')
    wrapper.unmount()
  })

  it('applies directives and refreshes the detail', async () => {
    const { wrapper } = await mountView('/instances')
    const before = calls.filter(
      (call) => call.method === 'GET' && call.url.includes('/api/instances/'),
    ).length

    await wrapper.get('[data-testid="log-level-input"]').setValue('doggy_dns=debug')
    await wrapper.get('[data-testid="log-level-apply"]').trigger('click')
    await flushPromises()

    const post = calls.find((call) => call.method === 'POST')
    expect(post?.url).toBe('/api/instances/n1/log-level')
    expect(post?.body).toEqual({ directives: 'doggy_dns=debug' })
    const after = calls.filter(
      (call) => call.method === 'GET' && call.url.includes('/api/instances/'),
    ).length
    expect(after).toBe(before + 1)
    wrapper.unmount()
  })

  it('resets to the startup filters', async () => {
    const { wrapper } = await mountView('/instances')

    await wrapper.get('[data-testid="log-level-reset"]').trigger('click')
    await flushPromises()

    const post = calls.find((call) => call.method === 'POST')
    expect(post?.body).toEqual({ reset: true })
    wrapper.unmount()
  })

  it('hides the editor when runtime control is disabled', async () => {
    const { wrapper } = await mountView('/instances', () => detailBody({ controlEnabled: false }))

    expect(wrapper.find('[data-testid="log-level-input"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('MCP_TOKEN')
    wrapper.unmount()
  })

  it('reports the message when the endpoint rejects the change', async () => {
    const { wrapper } = await mountView(
      '/instances',
      () => detailBody(),
      () => listed,
      () =>
        new Response(
          JSON.stringify({
            error: { code: 'invalid_request', message: 'directives must not be empty' },
          }),
          { status: 400, headers: { 'content-type': 'application/json' } },
        ),
    )

    await wrapper.get('[data-testid="log-level-input"]').setValue('debug')
    await wrapper.get('[data-testid="log-level-apply"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('directives must not be empty')
    wrapper.unmount()
  })
})

describe('InstancesView runtime log level race', () => {
  it('discards a failed change after the drawer moved to another instance', async () => {
    const second = {
      id: 'n2',
      host: 'dns-node-2',
      version: '0.1.0',
      online: true,
      lastSeen: 1,
      mcpReachable: true,
    }
    let resolvePost!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => {
      resolvePost = resolve
    })
    const detailFor = (url?: string) =>
      detailBody({
        instance: url?.includes('/n2') ? second : listed[0]!,
      })

    const { wrapper } = await mountView(
      '/instances',
      detailFor,
      () => [listed[0]!, second],
      () => pending,
    )

    await wrapper.get('[data-testid="log-level-input"]').setValue('debug')
    await wrapper.get('[data-testid="log-level-apply"]').trigger('click')
    await flushPromises()

    await wrapper.get('[aria-label="Close"]').trigger('click')
    await flushPromises()
    await wrapper.findAll('button.panel')[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('dns-node-2')
    // The new drawer is not stuck behind the old instance's in-flight write:
    // its controls must be enabled before that answer lands.
    expect(
      wrapper.get('[data-testid="log-level-reset"]').attributes('disabled'),
    ).toBeUndefined()

    resolvePost(
      new Response(
        JSON.stringify({
          error: { code: 'invalid_request', message: 'stale failure' },
        }),
        { status: 400, headers: { 'content-type': 'application/json' } },
      ),
    )
    await flushPromises()

    expect(wrapper.text()).not.toContain('stale failure')
    wrapper.unmount()
  })
})
