<script setup lang="ts">
import { computed, nextTick, ref, toRef, watch } from 'vue'
import { useRouter } from 'vue-router'
import { Bot, Maximize2, Minimize2, PanelRightClose, Send, Square, X } from '@lucide/vue'

import MessageBubble from '@/components/MessageBubble.vue'
import PromptChips from '@/components/PromptChips.vue'
import StatusDot from '@/components/StatusDot.vue'
import { useChatStore } from '@/stores/chat'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    telemetryDown: boolean
    rawQueryDown?: boolean
  }>(),
  { rawQueryDown: false },
)

const chat = useChatStore()
const ui = useUiStore()
const router = useRouter()
const draft = toRef(ui, 'composerDraft')
const list = ref<HTMLElement | null>(null)
// Following the answer must not fight a reader who scrolled up to re-read.
const pinned = ref(true)

async function send() {
  const content = draft.value.trim()
  if (!content || chat.running || props.telemetryDown) return
  pinned.value = true
  const sent = await chat.send(content)
  if (sent) draft.value = ''
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

function toggleFullscreen() {
  ui.setAiPanelMode(ui.aiPanelMode === 'fullscreen' ? 'docked' : 'fullscreen')
}

function reopenFailure() {
  const handoff = ui.handoff
  if (!handoff) return
  void router.push({
    name: 'failure',
    params: { traceId: handoff.traceId },
    query: handoff.windowMinutes ? { windowMinutes: String(handoff.windowMinutes) } : {},
  })
}

// A run grows one segment's text in place, so the message and segment counts alone
// would leave a long answer scrolling out of view.
const contentSize = computed(() => {
  const last = chat.messages[chat.messages.length - 1]
  if (!last) return 0
  return last.segments.reduce(
    (total, segment) =>
      segment.kind === 'tool'
        ? total + segment.args.length + (segment.result?.length ?? 0)
        : total + segment.text.length,
    0,
  )
})

function onScroll() {
  const el = list.value
  if (!el) return
  pinned.value = el.scrollHeight - el.scrollTop - el.clientHeight < 80
}

// A replaced transcript (new chat, session switch) must not inherit the per-message
// state of the one it replaced, so the bubbles are keyed to it.
const transcript = ref(0)
watch(
  () => chat.messages,
  () => {
    transcript.value += 1
  },
)

watch([() => chat.messages.length, contentSize], async () => {
  if (!pinned.value) return
  await nextTick()
  if (pinned.value && list.value) list.value.scrollTop = list.value.scrollHeight
})
</script>

<template>
  <section class="flex h-full min-h-0 flex-1 flex-col">
    <header class="flex items-center gap-2 border-b border-border px-4 py-3">
      <span class="grid h-8 w-8 shrink-0 place-items-center rounded-xl bg-accent/10 text-accent">
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
      <button
        class="focus-ring grid h-7 w-7 place-items-center rounded-lg border border-border text-muted hover:text-text"
        :aria-label="ui.aiPanelMode === 'fullscreen' ? '退出全屏' : '全屏显示'"
        data-test="ai-panel-fullscreen"
        @click="toggleFullscreen"
      >
        <Minimize2 v-if="ui.aiPanelMode === 'fullscreen'" :size="13" />
        <Maximize2 v-else :size="13" />
      </button>
      <button
        class="focus-ring grid h-7 w-7 place-items-center rounded-lg border border-border text-muted hover:text-text"
        aria-label="隐藏 AI 面板"
        data-test="ai-panel-hide"
        @click="ui.setAiPanelMode('hidden')"
      >
        <PanelRightClose :size="13" />
      </button>
    </header>

    <div
      v-if="ui.handoff"
      class="flex items-center gap-2 border-b border-border bg-surface-2/60 px-4 py-2 text-[11px]"
      data-testid="handoff-context"
    >
      <button
        class="focus-ring mono min-w-0 flex-1 truncate text-left text-accent hover:underline"
        data-test="reopen-failure"
        @click="reopenFailure"
      >
        来自 {{ ui.handoff.traceId }} · {{ ui.handoff.verdict }}
      </button>
      <button
        class="focus-ring grid h-6 w-6 shrink-0 place-items-center rounded-lg text-muted hover:text-text"
        aria-label="清除追问上下文"
        data-test="clear-handoff"
        @click="ui.clearHandoff()"
      >
        <X :size="12" />
      </button>
    </div>

    <div
      ref="list"
      data-testid="chat-list"
      class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4"
      @scroll.passive="onScroll"
    >
      <div v-if="chat.messages.length === 0" class="space-y-4">
        <div class="rounded-2xl border border-border bg-surface-2/50 p-4">
          <p class="text-sm text-text">先问结论，再下钻原始数据。</p>
          <p class="mt-1 text-xs leading-relaxed text-muted">
            我会先查 KPI 与失败分组；需要日志或 span 时再调用 OpenObserve 的只读查询工具。
          </p>
        </div>
        <PromptChips @select="pick" />
      </div>
      <MessageBubble
        v-for="(message, index) in chat.messages"
        :key="`${transcript}:${index}`"
        :message="message"
        :running="chat.running && index === chat.messages.length - 1"
      />
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
