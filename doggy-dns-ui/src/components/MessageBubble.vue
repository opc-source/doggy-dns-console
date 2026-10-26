<script setup lang="ts">
import ToolCallCard from './ToolCallCard.vue'
import type { ChatMessage } from '@/stores/chat'

defineProps<{ message: ChatMessage }>()
</script>

<template>
  <div class="flex flex-col gap-2" :class="message.role === 'user' ? 'items-end' : 'items-start'">
    <div
      class="max-w-[92%] whitespace-pre-wrap rounded-2xl px-3 py-2 text-sm leading-relaxed"
      :class="message.role === 'user' ? 'bg-accent text-bg' : 'border border-border bg-surface-2 text-text'"
    >
      <template v-if="message.content">{{ message.content }}</template>
      <span v-else class="inline-flex gap-1 text-muted">
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted" />
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted [animation-delay:120ms]" />
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted [animation-delay:240ms]" />
      </span>
    </div>
    <ToolCallCard v-for="call in message.toolCalls ?? []" :key="call.id" :call="call" class="w-full" />
  </div>
</template>
