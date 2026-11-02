<script setup lang="ts">
import { ref, watch } from 'vue'

import type { ThinkingSegment } from '@/lib/chatModel'

const props = defineProps<{ segment: ThinkingSegment; running?: boolean }>()

const expanded = ref(Boolean(props.running))
const chosen = ref(false)

watch(
  () => props.running,
  (isRunning) => {
    if (!chosen.value) expanded.value = Boolean(isRunning)
  },
)

// Once the reader picks a state by hand the auto-collapse stops touching this block.
function toggle() {
  chosen.value = true
  expanded.value = !expanded.value
}
</script>

<template>
  <details
    class="w-full max-w-[92%] rounded-xl border border-border/70 bg-surface-2/40 px-3 py-2 text-xs"
    :open="expanded"
  >
    <summary
      class="flex cursor-pointer list-none items-center gap-2 text-muted"
      @click.prevent="toggle"
    >
      <span class="capitalize">{{ running ? 'thinking' : 'thought' }}</span>
      <span class="text-[10px]">{{ segment.text.length }} 字</span>
      <span class="ml-auto text-[10px]">{{ expanded ? '收起' : '展开' }}</span>
    </summary>
    <p class="mt-2 whitespace-pre-wrap leading-relaxed text-muted">
      {{ segment.text }}
    </p>
  </details>
</template>
