<script setup lang="ts">
import { RouterView } from 'vue-router'

import SideRail from './SideRail.vue'
</script>

<template>
  <div class="flex h-full min-h-0 overflow-hidden">
    <SideRail />
    <main class="min-w-0 flex-1 overflow-y-auto">
      <RouterView />
    </main>
    <aside class="hidden w-[420px] shrink-0 border-l border-border bg-surface/40 xl:block">
      <slot name="ai">
        <div class="p-6 text-sm text-muted">AI Copilot</div>
      </slot>
    </aside>
  </div>
</template>
