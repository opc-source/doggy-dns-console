<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { Bot, Send, Square } from '@lucide/vue'

import MessageBubble from '@/components/MessageBubble.vue'
import PromptChips from '@/components/PromptChips.vue'
import StatusDot from '@/components/StatusDot.vue'
import { useChatStore } from '@/stores/chat'

const props = withDefaults(
  defineProps<{
    telemetryDown: boolean
    rawQueryDown?: boolean
  }>(),
  { rawQueryDown: false },
)

const chat = useChatStore()
const draft = ref('')
const list = ref<HTMLElement | null>(null)

async function send() {
  const content = draft.value.trim()
  if (!content || chat.running || props.telemetryDown) return
  draft.value = ''
  await chat.send(content)
}

function onKeydown(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
    event.preventDefault()
    void send()
  }
}

function pick(prompt: string) {
  draft.value = prompt
  void send()
}

watch(
  () => chat.messages.length,
  async () => {
    await nextTick()
    if (list.value) list.value.scrollTop = list.value.scrollHeight
  },
)
</script>

<template>
  <section class="flex h-full min-h-0 flex-col">
    <header class="flex items-center gap-3 border-b border-border px-4 py-3">
      <span class="grid h-8 w-8 place-items-center rounded-xl bg-accent/10 text-accent">
        <Bot :size="17" />
      </span>
      <div class="min-w-0 flex-1">
        <h2 class="text-sm font-medium">AI Copilot</h2>
        <p class="text-[11px] text-muted">grounded in OpenObserve + doggy-dns</p>
      </div>
      <StatusDot
        :status="telemetryDown ? 'error' : props.rawQueryDown ? 'warn' : 'ok'"
        :label="telemetryDown ? 'telemetry down' : props.rawQueryDown ? 'raw query down' : 'ready'"
      />
    </header>

    <div ref="list" class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4">
      <div v-if="chat.messages.length === 0" class="space-y-4">
        <div class="rounded-2xl border border-border bg-surface-2/50 p-4">
          <p class="text-sm text-text">先问结论，再下钻原始数据。</p>
          <p class="mt-1 text-xs leading-relaxed text-muted">
            我会先查 KPI 与失败分组；需要日志或 span 时再调用 OpenObserve 的只读查询工具。
          </p>
        </div>
        <PromptChips @select="pick" />
      </div>
      <MessageBubble v-for="(message, index) in chat.messages" :key="index" :message="message" />
    </div>

    <div class="border-t border-border px-4 py-3">
      <p v-if="telemetryDown" class="mb-2 rounded-lg border border-error/30 bg-error/10 px-2 py-1 text-[11px] text-error">
        OpenObserve 不可用，无法分析
      </p>
      <p v-else-if="props.rawQueryDown" class="mb-2 rounded-lg border border-warn/30 bg-warn/10 px-2 py-1 text-[11px] text-warn">
        OpenObserve 原始查询暂不可用，KPI 与失败分析仍可用
      </p>
      <p v-if="chat.error" class="mb-2 text-[11px] text-error">{{ chat.error }}</p>

      <div class="rounded-2xl border border-border bg-surface-2/60 p-2">
        <textarea
          v-model="draft"
          rows="3"
          class="w-full resize-none bg-transparent px-2 py-1 text-sm outline-none placeholder:text-muted disabled:opacity-40"
          placeholder="Ask about DNS health… (⌘/Ctrl + Enter)"
          :disabled="telemetryDown"
          @keydown="onKeydown"
        />
        <div class="flex items-center justify-between px-1">
          <span class="text-[10px] text-muted">only reads telemetry</span>
          <button
            v-if="chat.running"
            class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border text-muted hover:text-text"
            aria-label="Stop"
            @click="chat.stop()"
          >
            <Square :size="13" />
          </button>
          <button
            v-else
            class="focus-ring grid h-8 w-8 place-items-center rounded-xl bg-accent text-bg disabled:opacity-40"
            :disabled="telemetryDown || !draft.trim()"
            aria-label="Send"
            @click="send"
          >
            <Send :size="14" />
          </button>
        </div>
      </div>
    </div>
  </section>
</template>
