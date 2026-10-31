<script setup lang="ts">
import { computed } from 'vue'

import ErrorBanner from '@/components/ErrorBanner.vue'
import FailureDrawer from '@/components/FailureDrawer.vue'
import FailureList from '@/components/FailureList.vue'
import KpiCard from '@/components/KpiCard.vue'
import TrendChart from '@/components/TrendChart.vue'
import { useFailureList } from '@/composables/useFailureList'
import { useOverview } from '@/composables/useOverview'
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
const { data, error, loading, lastUpdated, windowMinutes, setWindow, refresh } = useOverview(60)
const { groups: failures, markers } = useFailureList(windowMinutes)

const windows = [
  { label: '1h', value: 60 },
  { label: '6h', value: 360 },
  { label: '24h', value: 1440 },
]

const successRate = computed(() => data.value?.successRate ?? null)
const errorMessage = computed(() => {
  if (!error.value) return ''
  if (data.value) return 'OpenObserve 暂不可用，展示的是上次成功数据'
  return '无法加载 Overview，请检查 OpenObserve 连接'
})
</script>

<template>
  <div class="mx-auto flex max-w-[1440px] flex-col gap-4 p-5 lg:p-7">
    <header class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="mono text-[11px] uppercase tracking-[0.18em] text-accent">doggy dns</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight lg:text-3xl">Overview</h1>
        <p class="mt-1 text-sm text-muted">Live DNS health across your fleet.</p>
      </div>
      <div class="flex items-center gap-2">
        <div class="flex rounded-xl border border-border bg-surface p-0.5">
          <button
            v-for="option in windows"
            :key="option.value"
            class="focus-ring rounded-lg px-3 py-1.5 text-xs transition-colors"
            :class="windowMinutes === option.value ? 'bg-surface-2 text-accent' : 'text-muted hover:text-text'"
            @click="setWindow(option.value)"
          >
            {{ option.label }}
          </button>
        </div>
        <button
          class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border bg-surface text-muted hover:text-text disabled:opacity-50"
          :disabled="loading"
          aria-label="Refresh"
          @click="refresh"
        >
          <span :class="{ 'animate-spin': loading }">⟳</span>
        </button>
      </div>
    </header>

    <ErrorBanner
      v-if="error"
      :message="errorMessage"
      :stale-at="data ? lastUpdated : null"
      @retry="refresh"
    />

    <section class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <KpiCard
        label="Instances online"
        :value="data ? `${data.instancesOnline} / ${data.instancesTotal}` : null"
        hint="MCP + telemetry"
      />
      <KpiCard label="Queries" :value="data?.queries ?? null" format="number" hint="selected window" />
      <KpiCard
        label="Success rate"
        :value="successRate"
        format="percent"
        :tone="successRate !== null && successRate < 0.99 ? 'warn' : 'ok'"
        hint="NXDOMAIN excluded"
      />
      <KpiCard label="P95 latency" :value="data?.p95LatencyMs ?? null" format="ms" hint="query duration" />
    </section>

    <section class="grid grid-cols-1 gap-3 xl:grid-cols-[1.5fr_1fr]">
      <TrendChart v-if="data" :series="data.series" />
      <div v-else class="panel h-[300px] animate-pulse bg-surface-2/40" />
      <FailureList
        :groups="failures"
        :markers="markers"
        @select="(group) => ui.openFailure(group.traceId)"
      />
    </section>

    <section class="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <div class="panel p-4">
        <h2 class="text-sm font-medium">What changed?</h2>
        <p class="mt-2 text-sm text-muted">
          AI analysis is docked on the right. Ask about the current window, or open a failure to
          hand its trace to the copilot.
        </p>
        <p class="mt-4 rounded-xl border border-border bg-surface-2 px-3 py-2 text-xs text-muted">
          Tip: open a failure to load its evidence — analysis starts automatically, and the
          share link restores it later.
        </p>
      </div>
      <div class="panel p-4">
        <h2 class="text-sm font-medium">Failure semantics</h2>
        <ul class="mt-2 space-y-1 text-xs text-muted">
          <li>SERVFAIL / REFUSED / FORMERR / timeout count as failures.</li>
          <li>NXDOMAIN is a miss, not a failure.</li>
          <li>Success rate is undefined when there is no traffic.</li>
        </ul>
      </div>
    </section>

    <FailureDrawer
      v-if="ui.drawerTraceId"
      :trace-id="ui.drawerTraceId"
      :window-minutes="windowMinutes"
      @close="ui.closeFailure()"
    />
  </div>
</template>
