<script setup lang="ts">
import TimeRangeControl from './TimeRangeControl.vue'
import type { FilterDraft } from '@/lib/failureFilter'

const props = defineProps<{
  modelValue: FilterDraft
  instances: string[]
}>()

const emit = defineEmits<{ 'update:modelValue': [draft: FilterDraft] }>()

const RESPONSE_CODES = ['SERVFAIL', 'REFUSED', 'FORMERR', 'NOERROR']

function patch(next: Partial<FilterDraft>) {
  emit('update:modelValue', { ...props.modelValue, ...next })
}
</script>

<template>
  <section class="panel flex flex-wrap items-end gap-3 px-4 py-3">
    <!-- The whole time section — presets, bracket, reset, hint — is shared with
         the dashboard and the instance drawer through this one control. The
         control has several root nodes, so in this flex row its roots would each
         become a direct flex item and the hint's `w-full` would force a wrap in
         the MIDDLE of the bar, shoving the three filter fields below it. Wrapping
         the control in its own flex container makes the whole time section a
         single flex item, so the hint stays on its own line above the fields. -->
    <div class="flex flex-wrap items-end gap-3">
      <TimeRangeControl
        :model-value="modelValue"
        @update:model-value="emit('update:modelValue', $event)"
      />
    </div>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      实例
      <select
        data-testid="instance-select"
        :value="modelValue.instance"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ instance: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部主机</option>
        <option v-for="host in instances" :key="host" :value="host">{{ host }}</option>
      </select>
    </label>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      响应码
      <select
        :value="modelValue.responseCode"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ responseCode: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部</option>
        <option v-for="code in RESPONSE_CODES" :key="code" :value="code">{{ code }}</option>
      </select>
    </label>

    <label class="flex min-w-40 flex-1 flex-col gap-1 text-[11px] text-muted">
      域名包含
      <input
        :value="modelValue.domain"
        placeholder="nacos.local"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @input="patch({ domain: ($event.target as HTMLInputElement).value })"
      />
    </label>
  </section>
</template>
