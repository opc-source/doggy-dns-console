<script setup lang="ts">
import { RouterView } from 'vue-router'

import AiPanel from './AiPanel.vue'
import SideRail from './SideRail.vue'
import { useConnections } from '@/composables/useConnections'
const { telemetryDown, rawQueryDown } = useConnections()
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
  </div>
</template>
