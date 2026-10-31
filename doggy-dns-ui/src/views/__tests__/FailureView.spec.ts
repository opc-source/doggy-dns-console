import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { routes } from '@/router'
import FailureView from '@/views/FailureView.vue'

const DrawerStub = defineComponent({
  name: 'FailureDrawer',
  props: {
    traceId: { type: String, required: true },
    windowMinutes: { type: Number, required: true },
    analysisId: { type: String, default: undefined },
  },
  emits: ['close'],
  template: '<div data-testid="drawer-stub" />',
})

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

describe('FailureView', () => {
  it('passes the trace, window and analysis from the deep link to the drawer', async () => {
    const { wrapper } = await mountView('/failures/t1?windowMinutes=360&analysis=a1')

    const drawer = wrapper.findComponent(DrawerStub)
    expect(drawer.exists()).toBe(true)
    expect(drawer.props('traceId')).toBe('t1')
    expect(drawer.props('windowMinutes')).toBe(360)
    expect(drawer.props('analysisId')).toBe('a1')
    expect(wrapper.find('[aria-label="Failure investigation"]').exists()).toBe(true)
    // AppShell already owns the page's <main> landmark.
    expect(wrapper.find('main').exists()).toBe(false)
  })

  it('falls back to a 60 minute window and no attempt for invalid query values', async () => {
    const { wrapper } = await mountView('/failures/t2?windowMinutes=abc')

    const drawer = wrapper.findComponent(DrawerStub)
    expect(drawer.props('traceId')).toBe('t2')
    expect(drawer.props('windowMinutes')).toBe(60)
    expect(drawer.props('analysisId')).toBeUndefined()
  })

  it.each([
    ['zero', '/failures/t3?windowMinutes=0'],
    ['a fractional value', '/failures/t3?windowMinutes=1.5'],
    ['a hexadecimal value', '/failures/t3?windowMinutes=0x10'],
    ['an exponent value', '/failures/t3?windowMinutes=1e3'],
  ])('falls back to 60 for %s', async (_label, path) => {
    const { wrapper } = await mountView(path)

    expect(wrapper.findComponent(DrawerStub).props('windowMinutes')).toBe(60)
  })

  it.each([
    ['below the backend minimum', '/failures/t6?windowMinutes=1', 5],
    ['above the backend maximum', '/failures/t6?windowMinutes=5000', 1440],
    ['beyond u32', '/failures/t6?windowMinutes=99999999999999999999', 1440],
  ])('clamps a window %s to the backend range', async (_label, path, expected) => {
    const { wrapper } = await mountView(path)

    expect(wrapper.findComponent(DrawerStub).props('windowMinutes')).toBe(expected)
  })

  it('trims whitespace around a positive integer window', async () => {
    const { wrapper } = await mountView('/failures/t4?windowMinutes=%20360%20')

    expect(wrapper.findComponent(DrawerStub).props('windowMinutes')).toBe(360)
  })

  it.each([
    ['an empty analysis value', '/failures/t5?analysis='],
    ['a whitespace analysis value', '/failures/t5?analysis=%20'],
  ])('omits the analysis id for %s', async (_label, path) => {
    const { wrapper } = await mountView(path)

    expect(wrapper.findComponent(DrawerStub).props('analysisId')).toBeUndefined()
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
