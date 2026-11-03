import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { defineComponent, type PropType } from 'vue'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { InvestigationFilter } from '@/api/types'
import { routes } from '@/router'
import FailureView from '@/views/FailureView.vue'

const DrawerStub = defineComponent({
  name: 'FailureDrawer',
  props: {
    traceId: { type: String, required: true },
    filter: { type: Object as PropType<InvestigationFilter>, required: true },
    analysisId: { type: String, default: undefined },
  },
  emits: ['close'],
  template: '<div data-testid="drawer-stub" />',
})

/** The bracket's width in whole minutes, which is what the old window prop was. */
function bracketMinutes(filter: unknown): number {
  const { from, to } = filter as InvestigationFilter
  return Math.round((to - from) / 60_000)
}

const wrappers: VueWrapper[] = []

async function mountView(path: string): Promise<{ wrapper: VueWrapper; router: Router }> {
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()

  const wrapper = mount(FailureView, {
    global: {
      plugins: [router, createPinia()],
      stubs: { FailureDrawer: DrawerStub },
    },
  })
  wrappers.push(wrapper)
  return { wrapper, router }
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
})

const NOW = Date.now()
const HOUR = 3_600_000

describe('FailureView', () => {
  it('passes an absolute bracket straight to the drawer', async () => {
    const from = NOW - 26 * HOUR
    const { wrapper } = await mountView(`/failures/t1?from=${from}&to=${NOW}`)

    const drawer = wrapper.findComponent(DrawerStub)
    expect(drawer.exists()).toBe(true)
    expect(drawer.props('traceId')).toBe('t1')
    const passed = drawer.props('filter')
    expect(passed.from).toBe(from)
    expect(passed.to).toBe(NOW)
    expect(passed.instance).toBeNull()
    expect(wrapper.find('[aria-label="Failure investigation"]').exists()).toBe(true)
    // AppShell already owns the page's <main> landmark.
    expect(wrapper.find('main').exists()).toBe(false)
  })

  it('expands a relative shortcut against the server clock without clamping', async () => {
    const { wrapper } = await mountView('/failures/t1?windowMinutes=10080')

    const passed = wrapper.findComponent(DrawerStub).props('filter')
    expect(bracketMinutes(passed)).toBe(10080)
  })

  it('keeps a seven-day bracket instead of folding it back to a day', async () => {
    const from = NOW - 7 * 24 * HOUR
    const { wrapper } = await mountView(`/failures/t1?from=${from}&to=${NOW}`)

    expect(wrapper.findComponent(DrawerStub).props('filter').from).toBe(from)
    expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(false)
  })

  it('carries domain, responseCode and instance into the request filter', async () => {
    const { wrapper } = await mountView(
      `/failures/t1?from=${NOW - 2 * HOUR}&to=${NOW}&domain=nacos&responseCode=SERVFAIL&instance=dns-node-2`,
    )

    const passed = wrapper.findComponent(DrawerStub).props('filter')
    expect(passed).toMatchObject({
      domain: 'nacos',
      responseCode: 'SERVFAIL',
      instance: 'dns-node-2',
    })
  })

  it('names an impossible bracket instead of querying something else', async () => {
    const { wrapper } = await mountView(`/failures/t1?from=${NOW}&to=${NOW}`)

    expect(wrapper.find('[data-testid="invalid-range"]').text()).toMatch(/早于|结束|分钟/)
    expect(wrapper.findComponent(DrawerStub).exists()).toBe(false)
  })

  it('still restores a deep-linked analysis id', async () => {
    const { wrapper } = await mountView(
      `/failures/t1?from=${NOW - 2 * HOUR}&to=${NOW}&analysis=inv-9`,
    )

    expect(wrapper.findComponent(DrawerStub).props('analysisId')).toBe('inv-9')
  })

  it('navigates back to the overview when the drawer closes', async () => {
    const { wrapper, router } = await mountView('/failures/t1?windowMinutes=360&analysis=a1')

    wrapper.findComponent(DrawerStub).vm.$emit('close')

    // The overview route is lazily imported, so the navigation settles later.
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe('overview'), {
      timeout: 5000,
    })
  })
})
