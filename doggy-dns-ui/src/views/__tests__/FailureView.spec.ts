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
  })

  it('falls back to a 60 minute window and no attempt for invalid query values', async () => {
    const { wrapper } = await mountView('/failures/t2?windowMinutes=abc')

    const drawer = wrapper.findComponent(DrawerStub)
    expect(drawer.props('traceId')).toBe('t2')
    expect(drawer.props('windowMinutes')).toBe(60)
    expect(drawer.props('analysisId')).toBeUndefined()
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
