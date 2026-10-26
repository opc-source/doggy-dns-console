<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'

defineProps<{
  message: string
  staleAt?: number | null
}>()

defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="flex items-center gap-3 rounded-xl border border-warn/30 bg-warn/10 px-4 py-3 text-sm text-warn">
    <TriangleAlert :size="16" />
    <span class="flex-1">{{ message }}</span>
    <span v-if="staleAt" class="mono text-[11px] opacity-70">
      last good {{ new Date(staleAt).toLocaleTimeString() }}
    </span>
    <button class="focus-ring rounded-lg border border-warn/40 px-2 py-1 text-xs hover:bg-warn/10" @click="$emit('retry')">
      Retry
    </button>
  </div>
</template>
