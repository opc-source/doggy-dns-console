import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import KpiCard from '@/components/KpiCard.vue'

describe('KpiCard', () => {
  it('renders dash for null success rate', () => {
    const wrapper = mount(KpiCard, {
      props: { label: 'Success rate', value: null, format: 'percent' },
    })
    expect(wrapper.text()).toContain('—')
  })

  it('formats a ratio as percent', () => {
    const wrapper = mount(KpiCard, {
      props: { label: 'Success rate', value: 0.997, format: 'percent' },
    })
    expect(wrapper.text()).toContain('99.7%')
  })
})
