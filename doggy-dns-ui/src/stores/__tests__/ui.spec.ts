import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { AI_PANEL_MODE_KEY, useUiStore } from '@/stores/ui'

describe('ui store AI panel mode', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
  })

  it('starts docked and persists only the non-fullscreen modes', () => {
    const ui = useUiStore()
    expect(ui.aiPanelMode).toBe('docked')

    ui.setAiPanelMode('hidden')
    expect(localStorage.getItem(AI_PANEL_MODE_KEY)).toBe('hidden')

    ui.setAiPanelMode('fullscreen')
    expect(localStorage.getItem(AI_PANEL_MODE_KEY)).toBe('hidden')
  })

  it('restores a persisted layout and falls back for junk', () => {
    localStorage.setItem(AI_PANEL_MODE_KEY, 'fullscreen')
    setActivePinia(createPinia())
    expect(useUiStore().aiPanelMode).toBe('docked')

    localStorage.setItem(AI_PANEL_MODE_KEY, 'hidden')
    setActivePinia(createPinia())
    expect(useUiStore().aiPanelMode).toBe('hidden')
  })

  it('opens docked on a wide layout and fullscreen on a narrow one', () => {
    const ui = useUiStore()

    ui.setAiPanelMode('hidden')
    ui.openAiPanel(true)
    expect(ui.aiPanelMode).toBe('docked')

    ui.setAiPanelMode('hidden')
    ui.openAiPanel(false)
    expect(ui.aiPanelMode).toBe('fullscreen')

    // A docked panel that has no room to dock still opens as an overlay.
    ui.setAiPanelMode('docked')
    ui.openAiPanel(false)
    expect(ui.aiPanelMode).toBe('fullscreen')
  })
})

describe('ui store handoff context', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
  })

  it('records the trace a follow-up came from until it is cleared', () => {
    const ui = useUiStore()
    expect(ui.handoff).toBeNull()

    ui.rememberHandoff({ traceId: 't-1', verdict: 'upstream_timeout', summary: '上游超时' })
    expect(ui.handoff).toMatchObject({ traceId: 't-1', verdict: 'upstream_timeout' })

    ui.clearHandoff()
    expect(ui.handoff).toBeNull()
  })
})
