import { mount, type VueWrapper } from '@vue/test-utils'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, describe, expect, it } from 'vitest'

import SideRail from '@/components/SideRail.vue'
import { routes } from '@/router'

const wrappers: VueWrapper[] = []

// The app's own route table, not a hand-copied one: a rail link pointing at a
// renamed destination must go red here, not keep passing against a stale stub.
async function mountRail(path: string): Promise<VueWrapper> {
  const router: Router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(SideRail, { global: { plugins: [router] } })
  wrappers.push(wrapper)
  return wrapper
}

function hrefFor(wrapper: VueWrapper, label: string): string {
  return wrapper.get(`a[aria-label="${label}"]`).attributes('href') ?? ''
}

describe('SideRail', () => {
  afterEach(() => {
    for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  })

  it('carries the current window into every nav destination', async () => {
    const wrapper = await mountRail('/failures?windowMinutes=360')

    // The bracket the operator was looking at must survive the click: Overview,
    // Instances and Sessions all read it out of their own URL.
    expect(hrefFor(wrapper, 'Overview')).toBe('/?windowMinutes=360')
    expect(hrefFor(wrapper, 'Instances')).toBe('/instances?windowMinutes=360')
    expect(hrefFor(wrapper, 'Sessions')).toBe('/sessions?windowMinutes=360')
    expect(hrefFor(wrapper, '失败列表')).toBe('/failures?windowMinutes=360')
    expect(hrefFor(wrapper, 'Doggy DNS home')).toBe('/?windowMinutes=360')
  })

  it('carries an absolute bracket as the pair it is', async () => {
    const wrapper = await mountRail('/instances?from=100&to=200')

    expect(hrefFor(wrapper, '失败列表')).toBe('/failures?from=100&to=200')
  })

  it('carries only a half bracket, so the next page refuses it the same way', async () => {
    const wrapper = await mountRail('/failures?from=100')

    // Completing or dropping the pair here would hide the draft the page has to
    // name: `validateDraft` shows 区间需要同时给出起止时间 behind this link.
    expect(hrefFor(wrapper, 'Overview')).toBe('/?from=100')
  })

  it('leaves the scope filters on the page that shows them', async () => {
    const wrapper = await mountRail(
      '/failures?windowMinutes=360&domain=nacos.local&responseCode=SERVFAIL&instance=dns-1',
    )

    // A filter with no control on the destination would keep narrowing rows the
    // operator cannot see or clear.
    expect(hrefFor(wrapper, 'Instances')).toBe('/instances?windowMinutes=360')
  })

  it('keeps the links clean when the page names no bracket', async () => {
    const wrapper = await mountRail('/sessions')

    expect(hrefFor(wrapper, 'Overview')).toBe('/')
    expect(hrefFor(wrapper, '失败列表')).toBe('/failures')
  })

  it('marks the open destination active with a bracket in the link', async () => {
    const wrapper = await mountRail('/failures?windowMinutes=360')

    // The link now carries a query, so this proves active matching stayed
    // query-free. `active-class` is the rail's own marker; `aria-current` cannot
    // stand in for it because vue-router only sets it on an exact match.
    expect(wrapper.get('a[aria-label="失败列表"]').classes()).toContain('bg-surface-2')
    expect(wrapper.get('a[aria-label="Overview"]').classes()).not.toContain('bg-surface-2')
  })

  it('keeps the list lit and drops the trace when the drawer is open', async () => {
    const wrapper = await mountRail('/failures/trace-1?windowMinutes=360')

    // The detail route is the list's child, so the destination stays lit and the
    // link goes back to the rows — the trace being read is not a range, and it
    // must not leak into Overview or Instances.
    expect(hrefFor(wrapper, '失败列表')).toBe('/failures?windowMinutes=360')
    expect(hrefFor(wrapper, 'Instances')).toBe('/instances?windowMinutes=360')
    expect(wrapper.get('a[aria-label="失败列表"]').classes()).toContain('bg-surface-2')
  })
})
