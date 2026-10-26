import { mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it } from 'vitest'

import AppShell from '@/components/AppShell.vue'
import { routes } from '@/router'

describe('AppShell', () => {
  it('renders the three nav destinations', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes })
    const wrapper = mount(AppShell, { global: { plugins: [router, createPinia()] } })
    await router.isReady()
    expect(wrapper.text()).toContain('Overview')
    expect(wrapper.text()).toContain('Instances')
    expect(wrapper.text()).toContain('Sessions')
  })

  it('opens the copilot drawer on narrow layouts', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes })
    const wrapper = mount(AppShell, {
      global: { plugins: [router, createPinia()] },
    })
    await router.isReady()
    await wrapper.find('[data-testid="mobile-ai-open"]').trigger('click')
    expect(wrapper.find('[data-testid="mobile-ai-panel"]').exists()).toBe(true)
  })
})
