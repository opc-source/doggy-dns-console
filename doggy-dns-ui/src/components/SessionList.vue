<script setup lang="ts">
import type { SessionSummary } from '@/api/types'

defineProps<{ sessions: SessionSummary[]; selected: string | null }>()
defineEmits<{ select: [id: string] }>()

function title(session: SessionSummary) {
  const user = session.events.find((event) => event.author === 'user')
  const text = user?.content?.parts?.map((part) => part.text).filter(Boolean).join(' ') ?? ''
  return text.slice(0, 48) || 'Untitled session'
}

function when(ts: number) {
  return new Date(ts * 1000).toLocaleString()
}
</script>

<template>
  <div class="panel flex h-full flex-col overflow-hidden">
    <header class="border-b border-border px-4 py-3">
      <h2 class="text-sm font-medium">Sessions</h2>
    </header>
    <div v-if="sessions.length === 0" class="px-4 py-10 text-center text-xs text-muted">
      No sessions yet.
    </div>
    <button
      v-for="session in sessions"
      :key="session.id"
      class="focus-ring border-b border-border/60 px-4 py-3 text-left transition-colors hover:bg-surface-2"
      :class="selected === session.id ? 'bg-surface-2' : ''"
      @click="$emit('select', session.id)"
    >
      <p class="truncate text-xs text-text">{{ title(session) }}</p>
      <p class="mono mt-1 text-[10px] text-muted">{{ when(session.lastUpdateTime) }}</p>
    </button>
  </div>
</template>
