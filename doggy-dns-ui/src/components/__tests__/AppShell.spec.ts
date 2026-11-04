import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import AppShell from '@/components/AppShell.vue'
import { routes } from '@/router'
import { AI_PANEL_MODE_KEY } from '@/stores/ui'

function setWide(wide: boolean) {
  // jsdom never matches media queries, so the layout switch is driven here.
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: wide && query.includes('1280px'),
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }))
}

const mounted: VueWrapper<unknown>[] = []

/**
 * The composer is disabled while the shell believes OpenObserve is down, and a disabled
 * control swallows the events a draft test fires at it. Other endpoints may keep failing:
 * only the health probe decides whether the panel accepts input.
 */
function stubHealthyConnections() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) =>
      path.endsWith('/api/connections')
        ? new Response(
            JSON.stringify({
              openobserve: { ok: true, latencyMs: 5 },
              openobserveMcp: { ok: true, latencyMs: 5 },
              mcp: { reachable: 1, total: 1 },
              model: { configured: true },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          )
        : Promise.reject(new Error('offline')),
    ),
  )
}

async function mountShell() {
  const router = createRouter({ history: createMemoryHistory(), routes })
  // Attached so window-level listeners see events bubbling out of the shell.
  const wrapper = mount(AppShell, {
    attachTo: document.body,
    global: { plugins: [router, createPinia()] },
  })
  mounted.push(wrapper)
  await router.isReady()
  return wrapper
}

describe('AppShell', () => {
  beforeEach(() => {
    localStorage.clear()
    setWide(true)
  })

  afterEach(() => {
    for (const wrapper of mounted.splice(0)) wrapper.unmount()
    vi.unstubAllGlobals()
    localStorage.clear()
  })

  it('renders the four nav destinations', async () => {
    const wrapper = await mountShell()
    expect(wrapper.text()).toContain('Overview')
    expect(wrapper.text()).toContain('Instances')
    expect(wrapper.text()).toContain('Sessions')
    expect(wrapper.text()).toContain('失败列表')
  })

  it('docks the panel on a wide layout', async () => {
    const wrapper = await mountShell()
    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(false)
  })

  it('hides the panel behind a floating button and brings it back', async () => {
    const wrapper = await mountShell()
    await wrapper.find('[data-test="ai-panel-hide"]').trigger('click')

    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(false)
    expect(localStorage.getItem(AI_PANEL_MODE_KEY)).toBe('hidden')

    await wrapper.find('[data-testid="ai-panel-open"]').trigger('click')
    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(true)
  })

  it('expands the panel over the dashboard and returns with Escape', async () => {
    const wrapper = await mountShell()
    await wrapper.find('[data-test="ai-panel-fullscreen"]').trigger('click')

    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(false)
    // A transient reading mode must not overwrite the remembered layout.
    expect(localStorage.getItem(AI_PANEL_MODE_KEY)).not.toBe('fullscreen')

    await wrapper.find('[data-testid="ai-panel-overlay"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(true)
  })

  it('returns a fullscreen panel with Escape pressed anywhere', async () => {
    const wrapper = await mountShell()
    await wrapper.find('[data-test="ai-panel-fullscreen"]').trigger('click')
    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(true)

    // The reader may have moved focus to the rail or the dashboard to compare.
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(true)
  })

  it('leaves the side rail reachable while the panel is fullscreen', async () => {
    const wrapper = await mountShell()
    await wrapper.find('[data-test="ai-panel-fullscreen"]').trigger('click')

    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(true)
    expect(wrapper.find('nav').exists()).toBe(true)
    // aria-modal would tell assistive tech to hide the rail the layout keeps on screen.
    expect(wrapper.get('[data-testid="ai-panel-overlay"]').attributes('aria-modal')).toBe('false')
  })

  it('keeps the draft when the panel changes size', async () => {
    stubHealthyConnections()
    const wrapper = await mountShell()
    const textarea = wrapper.find('textarea')
    await flushPromises()
    expect(textarea.attributes('disabled')).toBeUndefined()
    await textarea.setValue('还没发出去的问题')
    const aside = wrapper.get('[data-testid="ai-panel-docked"]').element

    await wrapper.find('[data-test="ai-panel-fullscreen"]').trigger('click')

    // The store keeps the draft, so only node identity proves the panel was not remounted
    // — a remount would also reset the reading position and every expanded tool card.
    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(true)
    expect(wrapper.get('[data-testid="ai-panel-overlay"]').element).toBe(aside)
    expect(wrapper.find('textarea').element).toBe(textarea.element)
    expect(wrapper.find('textarea').element.value).toBe('还没发出去的问题')
  })

  it('keeps the draft across hide and reopen', async () => {
    stubHealthyConnections()
    const wrapper = await mountShell()
    await wrapper.find('textarea').setValue('还没发出去的问题')

    await wrapper.find('[data-test="ai-panel-hide"]').trigger('click')
    await wrapper.find('[data-testid="ai-panel-open"]').trigger('click')

    expect(wrapper.find('textarea').element.value).toBe('还没发出去的问题')
  })

  it('opens an overlay instead of a dock on a narrow layout', async () => {
    localStorage.setItem(AI_PANEL_MODE_KEY, 'docked')
    setWide(false)
    const wrapper = await mountShell()

    expect(wrapper.find('[data-testid="ai-panel-docked"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(false)

    await wrapper.find('[data-testid="ai-panel-open"]').trigger('click')
    expect(wrapper.find('[data-testid="ai-panel-overlay"]').exists()).toBe(true)
  })

  it('returns focus to the opener when the narrow overlay closes', async () => {
    localStorage.setItem(AI_PANEL_MODE_KEY, 'docked')
    setWide(false)
    const wrapper = await mountShell()

    const opener = wrapper.get('[data-testid="ai-panel-open"]')
    await opener.trigger('click')
    const overlay = wrapper.get('[data-testid="ai-panel-overlay"]')
    // The overlay is a reading surface, so focus moves into it on open.
    expect(document.activeElement).toBe(overlay.element)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    // The watcher returns focus one tick after the overlay unmounts, so the button it
    // hands focus to exists again.
    await flushPromises()
    await wrapper.vm.$nextTick()

    // Closing unmounts the focused element; without this the reader is dropped at the
    // top of the document and has to tab through the whole dashboard again.
    expect(document.activeElement).toBe(wrapper.get('[data-testid="ai-panel-open"]').element)
  })
})
