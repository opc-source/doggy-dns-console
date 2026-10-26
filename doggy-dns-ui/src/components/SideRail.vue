<script setup lang="ts">
import { LayoutDashboard, Server, History, PawPrint } from '@lucide/vue'
import { RouterLink } from 'vue-router'

const links = [
  { to: '/', label: 'Overview', icon: LayoutDashboard },
  { to: '/instances', label: 'Instances', icon: Server },
  { to: '/sessions', label: 'Sessions', icon: History },
]
</script>

<template>
  <nav class="flex w-16 shrink-0 flex-col items-center gap-2 border-r border-border bg-surface/50 px-2 py-4">
    <RouterLink
      to="/"
      class="mb-5 grid h-10 w-10 place-items-center rounded-xl bg-accent text-bg"
      aria-label="Doggy DNS home"
    >
      <PawPrint :size="20" :stroke-width="2.4" />
    </RouterLink>

    <RouterLink
      v-for="link in links"
      :key="link.to"
      :to="link.to"
      class="focus-ring group relative grid h-10 w-10 place-items-center rounded-xl text-muted transition-colors hover:bg-surface-2 hover:text-text"
      :aria-label="link.label"
      active-class="bg-surface-2 text-accent"
    >
      <component :is="link.icon" :size="18" />
      <span class="pointer-events-none absolute left-12 z-20 hidden rounded-lg border border-border bg-surface-2 px-2 py-1 text-xs text-text group-hover:block">
        {{ link.label }}
      </span>
    </RouterLink>

    <div class="mt-auto grid h-10 w-10 place-items-center rounded-full border border-border text-[10px] text-muted">DNS</div>
  </nav>
</template>
