<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, RouterLink } from 'vue-router'
import { LayoutDashboard, Server, History, PawPrint, TriangleAlert } from '@lucide/vue'

import { pickRangeQuery } from '@/lib/failureFilter'

const links = [
  { to: '/', label: 'Overview', icon: LayoutDashboard },
  { to: '/instances', label: 'Instances', icon: Server },
  { to: '/failures', label: '失败列表', icon: TriangleAlert },
  { to: '/sessions', label: 'Sessions', icon: History },
]

// `LocationQuery` arrays may hold nulls, which `RawQuery` does not model, so this
// is the same widen-free cast the views apply to `route.query`.
type RangeQuery = Parameters<typeof pickRangeQuery>[0]

const route = useRoute()
// Every page reads its bracket from the URL, so the rail hands the current one
// over instead of letting the destination fall back to the default window.
const range = computed(() => pickRangeQuery(route.query as RangeQuery))
</script>

<template>
  <nav class="flex w-16 shrink-0 flex-col items-center gap-2 border-r border-border bg-surface/50 px-2 py-4">
    <RouterLink
      :to="{ path: '/', query: range }"
      class="mb-5 grid h-10 w-10 place-items-center rounded-xl bg-accent text-bg"
      aria-label="Doggy DNS home"
    >
      <PawPrint :size="20" :stroke-width="2.4" />
    </RouterLink>

    <RouterLink
      v-for="link in links"
      :key="link.to"
      :to="{ path: link.to, query: range }"
      class="focus-ring group relative grid h-10 w-10 place-items-center rounded-xl text-muted transition-colors hover:bg-surface-2 hover:text-text"
      :aria-label="link.label"
      active-class="bg-surface-2 text-accent"
    >
      <component :is="link.icon" :size="18" />
      <!--
        `whitespace-nowrap` is load-bearing: this box is `absolute left-12` inside a
        40px-wide link, so its shrink-to-fit width collapses to min-content — one Han
        character. nowrap makes min-content the whole line.
      -->
      <span
        data-testid="rail-tip"
        class="pointer-events-none absolute left-12 z-20 hidden whitespace-nowrap rounded-lg border border-border bg-surface-2 px-2 py-1 text-xs text-text group-hover:block"
      >
        {{ link.label }}
      </span>
    </RouterLink>

    <div class="mt-auto grid h-10 w-10 place-items-center rounded-full border border-border text-[10px] text-muted">DNS</div>
  </nav>
</template>
