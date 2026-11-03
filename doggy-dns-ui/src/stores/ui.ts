import { ref } from 'vue'
import { defineStore } from 'pinia'

export const AI_PANEL_MODE_KEY = 'doggy-dns.ai-panel-mode'

export type AiPanelMode = 'docked' | 'hidden' | 'fullscreen'

export interface HandoffContext {
  traceId: string
  verdict: string
  summary: string
  /** Set together when the investigation was framed absolutely. */
  fromMs?: number
  toMs?: number
}

function restoredMode(): AiPanelMode {
  // A fullscreen panel is a transient reading mode, never a persisted layout.
  const stored = localStorage.getItem(AI_PANEL_MODE_KEY)
  return stored === 'hidden' || stored === 'docked' ? stored : 'docked'
}

export const useUiStore = defineStore('ui', () => {
  const drawerTraceId = ref<string | null>(null)
  const aiPanelMode = ref<AiPanelMode>(restoredMode())
  const handoff = ref<HandoffContext | null>(null)
  // Hiding the panel unmounts it, so an unsent question has to live outside it.
  const composerDraft = ref('')

  function openFailure(traceId: string | null) {
    drawerTraceId.value = traceId
  }

  function closeFailure() {
    drawerTraceId.value = null
  }

  function persist(mode: AiPanelMode) {
    if (mode === 'fullscreen') return
    try {
      localStorage.setItem(AI_PANEL_MODE_KEY, mode)
    } catch {
      // A blocked storage (private mode) only costs layout memory.
    }
  }

  function setAiPanelMode(mode: AiPanelMode) {
    aiPanelMode.value = mode
    persist(mode)
  }

  function openAiPanel(isWide: boolean) {
    setAiPanelMode(isWide ? 'docked' : 'fullscreen')
  }

  function rememberHandoff(context: HandoffContext) {
    handoff.value = context
  }

  function clearHandoff() {
    handoff.value = null
  }

  return {
    drawerTraceId,
    aiPanelMode,
    handoff,
    composerDraft,
    openFailure,
    closeFailure,
    setAiPanelMode,
    openAiPanel,
    rememberHandoff,
    clearHandoff,
  }
})
