<script setup lang="ts">
import { computed } from 'vue'

import { sanitizeMarkdown } from '@/lib/markdown'

const props = defineProps<{ text: string }>()

function jsonBlock(text: string): string | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return null
  try {
    const parsed: unknown = JSON.parse(trimmed)
    return typeof parsed === 'object' && parsed !== null ? JSON.stringify(parsed, null, 2) : null
  } catch {
    return null
  }
}

const formattedJson = computed(() => jsonBlock(props.text))

const html = computed(() => sanitizeMarkdown(props.text))
</script>

<template>
  <div v-if="!props.text.trim()" />
  <pre v-else-if="formattedJson" class="md-json">{{ formattedJson }}</pre>
  <!-- The only v-html in the app: `html` is DOMPurify-sanitised against a tag and attribute allowlist. -->
  <div v-else class="md" v-html="html" />
</template>

<style scoped>
.md :is(h1, h2, h3, h4, h5, h6) {
  margin: 0.6em 0 0.3em;
  font-size: 0.95em;
  font-weight: 600;
}
.md :is(p, ul, ol, blockquote, pre, table) {
  margin: 0.35em 0;
}
.md ul {
  list-style: disc;
  padding-left: 1.2em;
}
.md ol {
  list-style: decimal;
  padding-left: 1.2em;
}
.md li + li {
  margin-top: 0.15em;
}
.md a {
  color: var(--color-accent);
  text-decoration: underline;
}
.md blockquote {
  border-left: 2px solid var(--color-border);
  padding-left: 0.7em;
  color: var(--color-muted);
}
.md :is(code, .md-json) {
  font-family: var(--font-mono);
  font-size: 0.85em;
}
.md code {
  background: color-mix(in oklab, var(--color-surface-2) 70%, transparent);
  border-radius: 0.25rem;
  padding: 0.05em 0.3em;
}
.md pre,
.md-json {
  background: color-mix(in oklab, var(--color-surface-2) 70%, transparent);
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  overflow-x: auto;
  padding: 0.5em 0.65em;
  white-space: pre;
}
.md pre code {
  background: transparent;
  padding: 0;
}
.md-json {
  font-family: var(--font-mono);
  font-size: 0.85em;
  white-space: pre-wrap;
  word-break: break-word;
}
.md table {
  border-collapse: collapse;
  width: 100%;
}
.md :is(th, td) {
  border: 1px solid var(--color-border);
  padding: 0.2em 0.4em;
  text-align: left;
}
</style>
