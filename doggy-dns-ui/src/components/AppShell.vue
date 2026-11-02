<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RouterView } from 'vue-router'
import { Bot } from '@lucide/vue'

import AiPanel from './AiPanel.vue'
import SideRail from './SideRail.vue'
import { useConnections } from '@/composables/useConnections'
import { useMediaQuery } from '@/composables/useMediaQuery'
import { useUiStore } from '@/stores/ui'

const { telemetryDown, rawQueryDown } = useConnections()
const ui = useUiStore()
const isWide = useMediaQuery('(min-width: 1280px)')
const panel = ref<HTMLElement | null>(null)
const opener = ref<HTMLButtonElement | null>(null)

const showDock = computed(() => ui.aiPanelMode === 'docked' && isWide.value)
const showOverlay = computed(() => ui.aiPanelMode === 'fullscreen')
const showFloatingButton = computed(() => !showDock.value && !showOverlay.value)

// The overlay is a reading surface, so focus moves into it on open.
watch(showOverlay, async (open) => {
  if (open) await nextTick(() => panel.value?.focus())
})

// On a narrow layout closing the overlay unmounts the element that held focus, so the
// reader would be dropped at the top of the document; hand focus back to the opener.
watch(showFloatingButton, async (visible) => {
  if (visible) await nextTick(() => opener.value?.focus())
})

function onKeydown(event: KeyboardEvent) {
  // Escape leaves the reading mode wherever the reader moved focus.
  if (event.key === 'Escape' && ui.aiPanelMode === 'fullscreen') ui.setAiPanelMode('docked')
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div class="flex h-full min-h-0 overflow-hidden">
    <SideRail />
    <main class="min-w-0 flex-1 overflow-y-auto">
      <RouterView />
    </main>

    <!--
      One element for both visible modes: toggling fullscreen must not remount the
      panel, or the draft and the reading position would vanish with it.
    -->
    <aside
      v-if="showDock || showOverlay"
      ref="panel"
      :class="
        showOverlay
          ? 'fixed inset-y-0 left-16 right-0 z-50 flex bg-surface'
          : 'w-[420px] shrink-0 border-l border-border bg-surface/40'
      "
      :data-testid="showOverlay ? 'ai-panel-overlay' : 'ai-panel-docked'"
      :role="showOverlay ? 'dialog' : undefined"
      :aria-modal="showOverlay ? 'false' : undefined"
      :tabindex="showOverlay ? -1 : undefined"
      aria-label="AI Copilot"
    >
      <AiPanel :telemetry-down="telemetryDown" :raw-query-down="rawQueryDown" />
    </aside>

    <button
      v-if="showFloatingButton"
      ref="opener"
      class="focus-ring fixed bottom-5 right-5 z-30 grid h-12 w-12 place-items-center rounded-2xl bg-accent text-bg shadow-2xl"
      data-testid="ai-panel-open"
      aria-label="Open AI Copilot"
      @click="ui.openAiPanel(isWide)"
    >
      <Bot :size="20" />
    </button>
  </div>
</template>
