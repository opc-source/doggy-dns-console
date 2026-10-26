<script setup lang="ts">
import { Server } from '@lucide/vue'

import type { Instance } from '@/api/types'
import StatusDot from '@/components/StatusDot.vue'

defineProps<{ instance: Instance }>()
defineEmits<{ select: [instance: Instance] }>()

function ago(ts: number) {
  const seconds = Math.max(0, Math.round((Date.now() - ts) / 1000))
  if (seconds < 60) return `${seconds}s ago`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`
  return `${Math.floor(seconds / 3600)}h ago`
}
</script>

<template>
  <button
    class="focus-ring panel w-full p-4 text-left transition-colors hover:border-accent/30"
    :class="instance.online ? '' : 'opacity-65'"
    @click="$emit('select', instance)"
  >
    <div class="flex items-start gap-3">
      <span class="grid h-9 w-9 place-items-center rounded-xl border border-border bg-surface-2 text-muted">
        <Server :size="16" />
      </span>
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <p class="mono truncate text-sm text-text">{{ instance.host }}</p>
          <StatusDot :status="instance.online ? 'ok' : 'error'" :label="instance.online ? 'online' : 'offline'" />
        </div>
        <p class="mt-1 text-[11px] text-muted">
          {{ instance.version ?? 'unknown version' }} · seen {{ ago(instance.lastSeen) }}
        </p>
        <div class="mt-3 flex flex-wrap gap-1.5">
          <span
            class="rounded-full border px-2 py-0.5 text-[10px]"
            :class="instance.mcpReachable ? 'border-ok/30 text-ok' : 'border-warn/30 text-warn'"
          >
            {{ instance.mcpReachable ? 'MCP reachable' : 'MCP unreachable' }}
          </span>
          <span v-if="!instance.mcpReachable && instance.online" class="rounded-full border border-border px-2 py-0.5 text-[10px] text-muted">
            telemetry only
          </span>
        </div>
      </div>
    </div>
  </button>
</template>
