<script setup lang="ts">
import MarkdownBlock from './MarkdownBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import ToolCallCard from './ToolCallCard.vue'
import type { ChatMessage } from '@/lib/chatModel'

defineProps<{ message: ChatMessage; running?: boolean }>()
</script>

<template>
  <div class="flex flex-col gap-2" :class="message.role === 'user' ? 'items-end' : 'items-start'">
    <template v-if="message.role === 'user'">
      <div
        v-for="(segment, index) in message.segments"
        :key="index"
        data-segment="text"
        class="max-w-[92%] whitespace-pre-wrap rounded-2xl bg-accent px-3 py-2 text-sm leading-relaxed text-bg"
      >
        {{ segment.kind === 'text' ? segment.text : '' }}
      </div>
    </template>
    <template v-else>
      <template v-for="(segment, index) in message.segments" :key="index">
        <ThinkingBlock
          v-if="segment.kind === 'thinking'"
          data-segment="thinking"
          :segment="segment"
          :running="running"
        />
        <ToolCallCard
          v-else-if="segment.kind === 'tool'"
          data-segment="tool"
          :call="segment"
          class="w-full max-w-[92%]"
        />
        <div
          v-else
          data-segment="text"
          class="max-w-[92%] rounded-2xl border border-border bg-surface-2 px-3 py-2 text-sm leading-relaxed text-text"
        >
          <MarkdownBlock :text="segment.text" />
        </div>
      </template>
      <span
        v-if="message.segments.length === 0"
        class="inline-flex gap-1 rounded-2xl border border-border bg-surface-2 px-3 py-2 text-muted"
        data-test="typing-indicator"
      >
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted" />
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted [animation-delay:120ms]" />
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-muted [animation-delay:240ms]" />
      </span>
    </template>
  </div>
</template>
