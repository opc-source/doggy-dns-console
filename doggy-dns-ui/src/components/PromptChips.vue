<script setup lang="ts">
const prompts = [
  '分析最近 1 小时失败',
  '哪些域名失败最多',
  '有没有节点掉线',
  '解释这次 SERVFAIL',
]

const emit = defineEmits<{ select: [prompt: string] }>()
</script>

<template>
  <div class="flex flex-wrap gap-2">
    <button
      v-for="prompt in prompts"
      :key="prompt"
      class="focus-ring rounded-full border border-border bg-surface-2 px-3 py-1.5 text-[11px] text-muted transition-colors hover:border-accent/40 hover:text-text"
      @click="emit('select', prompt)"
    >
      {{ prompt }}
    </button>
  </div>
</template>
