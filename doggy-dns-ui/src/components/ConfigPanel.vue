<script setup lang="ts">
defineProps<{ toml: string }>()
</script>

<template>
  <pre class="mono max-h-[320px] overflow-auto rounded-xl border border-border bg-bg/70 p-3 text-[10px] leading-relaxed text-muted">{{ toml }}</pre>
</template>
