<script setup lang="ts">
import { ref } from 'vue'
import { RouterView } from 'vue-router'
import { Bot, X } from '@lucide/vue'

import AiPanel from './AiPanel.vue'
import SideRail from './SideRail.vue'
import { useConnections } from '@/composables/useConnections'
const { telemetryDown, rawQueryDown } = useConnections()
const mobileAiOpen = ref(false)
</script>

<template>
  <div class="flex h-full min-h-0 overflow-hidden">
    <SideRail />
    <main class="min-w-0 flex-1 overflow-y-auto">
      <RouterView />
    </main>
    <aside class="hidden w-[420px] shrink-0 border-l border-border bg-surface/40 xl:block">
      <AiPanel :telemetry-down="telemetryDown" :raw-query-down="rawQueryDown" />
    </aside>

    <button
      class="focus-ring fixed bottom-5 right-5 z-30 grid h-12 w-12 place-items-center rounded-2xl bg-accent text-bg shadow-2xl xl:hidden"
      data-testid="mobile-ai-open"
      aria-label="Open AI Copilot"
      @click="mobileAiOpen = true"
    >
      <Bot :size="20" />
    </button>

    <div
      v-if="mobileAiOpen"
      class="fixed inset-0 z-50 bg-surface xl:hidden"
      data-testid="mobile-ai-panel"
      role="dialog"
      aria-modal="true"
      aria-label="AI Copilot"
    >
      <button
        class="focus-ring absolute right-4 top-4 z-10 grid h-8 w-8 place-items-center rounded-xl border border-border bg-surface-2 text-muted"
        aria-label="Close AI Copilot"
        @click="mobileAiOpen = false"
      >
        <X :size="15" />
      </button>
      <AiPanel :telemetry-down="telemetryDown" :raw-query-down="rawQueryDown" />
    </div>
  </div>
</template>
