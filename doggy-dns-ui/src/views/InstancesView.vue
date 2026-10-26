<script setup lang="ts">
import { computed, ref, toRef } from 'vue'
import { RefreshCw, X } from '@lucide/vue'

import ConfigPanel from '@/components/ConfigPanel.vue'
import ErrorBanner from '@/components/ErrorBanner.vue'
import InstanceCard from '@/components/InstanceCard.vue'
import StatusDot from '@/components/StatusDot.vue'
import { useInstanceDetail, useInstances } from '@/composables/useInstances'
import type { Instance } from '@/api/types'

const { instances, error, refresh } = useInstances()
const selected = ref<string | null>(null)
const selectedId = toRef(selected)
const { detail, error: detailError, loading } = useInstanceDetail(selectedId)

const online = computed(() => instances.value.filter((item) => item.online).length)

function select(instance: Instance) {
  selected.value = instance.id
}
</script>

<template>
  <div class="mx-auto flex max-w-[1440px] flex-col gap-4 p-5 lg:p-7">
    <header class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="mono text-[11px] uppercase tracking-[0.18em] text-accent">fleet</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight lg:text-3xl">Instances</h1>
        <p class="mt-1 text-sm text-muted">{{ online }} online · {{ instances.length }} known</p>
      </div>
      <button class="focus-ring flex items-center gap-2 rounded-xl border border-border bg-surface px-3 py-2 text-xs text-muted hover:text-text" @click="refresh">
        <RefreshCw :size="13" /> Refresh
      </button>
    </header>

    <ErrorBanner v-if="error" :message="error" @retry="refresh" />

    <div v-if="instances.length === 0" class="panel p-10 text-center">
      <p class="text-sm text-text">还没有发现 doggy-dns 实例</p>
      <p class="mt-1 text-xs text-muted">配置 MCP_ENDPOINTS，或确认 OpenObserve 已收到 host.name。</p>
    </div>
    <section v-else class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      <InstanceCard v-for="instance in instances" :key="instance.id" :instance="instance" @select="select" />
    </section>

    <div
      v-if="selected"
      class="fixed inset-0 z-40 flex justify-end bg-black/40 backdrop-blur-[2px]"
      tabindex="-1"
      @click.self="selected = null"
      @keydown.esc="selected = null"
    >
      <aside
        class="flex h-full w-full max-w-[560px] flex-col border-l border-border bg-surface shadow-2xl"
        role="dialog"
        aria-modal="true"
        aria-label="Instance detail"
      >
        <header class="flex items-start justify-between border-b border-border px-4 py-4">
          <div>
            <p class="mono text-[10px] uppercase tracking-[0.16em] text-accent">instance</p>
            <h2 class="mono mt-1 text-sm text-text">{{ selected }}</h2>
            <div class="mt-2 flex items-center gap-2">
              <StatusDot :status="detail?.instance.online ? 'ok' : 'error'" :label="detail?.instance.online ? 'online' : 'offline'" />
              <span v-if="detail" class="mono text-[10px] text-muted">{{ detail.listenAddr }}</span>
            </div>
          </div>
          <button class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border text-muted hover:text-text" aria-label="Close" @click="selected = null">
            <X :size="15" />
          </button>
        </header>

        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
          <p v-if="loading" class="text-xs text-muted">Loading instance…</p>
          <p v-else-if="detailError" class="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{{ detailError }}</p>
          <template v-else-if="detail">
            <section class="grid grid-cols-2 gap-2">
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">uptime</p>
                <p class="mt-1 text-sm">{{ Math.floor((detail.uptimeSeconds ?? 0) / 3600) }}h</p>
              </div>
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">protocols</p>
                <p class="mono mt-1 text-sm">{{ detail.protocols.join(' · ') || '—' }}</p>
              </div>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Plugins</h3>
              <div class="space-y-2">
                <article v-for="plugin in detail.plugins" :key="plugin.order" class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <div class="flex items-center gap-2">
                    <span class="mono text-xs">{{ plugin.kind }}</span>
                    <StatusDot :status="plugin.enabled ? 'ok' : 'idle'" :label="plugin.enabled ? 'enabled' : 'disabled'" />
                  </div>
                  <pre class="mono mt-2 whitespace-pre-wrap break-all text-[10px] text-muted">{{ JSON.stringify(plugin.details) }}</pre>
                </article>
              </div>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Config (read-only)</h3>
              <ConfigPanel v-if="detail.configToml" :toml="detail.configToml" />
              <p v-else class="text-xs text-muted">MCP did not return config.</p>
            </section>
          </template>
        </div>
      </aside>
    </div>
  </div>
</template>
