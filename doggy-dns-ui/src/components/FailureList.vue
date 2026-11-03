<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'

import type { FailureGroup } from '@/api/types'

import FailureRow from './FailureRow.vue'

const props = withDefaults(
  defineProps<{
    groups: FailureGroup[]
    markers?: Set<string>
    /** Rows to render; 0 means "all of them". */
    limit?: number
    /** Render the "查看全部" link to the failure list page. */
    seeAll?: boolean
    title?: string
    subtitle?: string
    /** One-line rows for the overview card; the page renders every field. */
    dense?: boolean
  }>(),
  {
    limit: 5,
    seeAll: false,
    title: 'Top failures',
    subtitle: 'last window',
    dense: true,
  },
)

const emit = defineEmits<{ select: [group: FailureGroup] }>()

const visibleGroups = computed(() =>
  props.limit && props.limit > 0 ? props.groups.slice(0, props.limit) : props.groups,
)
</script>

<template>
  <div class="panel overflow-hidden">
    <header class="flex items-center justify-between border-b border-border px-4 py-3">
      <h2 class="text-sm font-medium">{{ title }}</h2>
      <div class="flex items-center gap-2">
        <span class="text-xs text-muted">{{ subtitle }}</span>
        <RouterLink
          v-if="seeAll"
          data-testid="see-all"
          :to="{ name: 'failures-list' }"
          class="focus-ring rounded-md px-2 py-1 text-xs text-accent hover:underline"
          >查看全部</RouterLink
        >
      </div>
    </header>
    <div v-if="groups.length === 0" class="px-4 py-10 text-center text-sm text-muted">
      No failures in this window.
    </div>
    <FailureRow
      v-for="group in visibleGroups"
      :key="group.key"
      :group="group"
      :analyzed="markers?.has(group.traceId ?? '') ?? false"
      :dense="dense"
      @select="emit('select', $event)"
    />
  </div>
</template>
