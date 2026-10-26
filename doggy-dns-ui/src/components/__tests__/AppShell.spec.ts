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
})
