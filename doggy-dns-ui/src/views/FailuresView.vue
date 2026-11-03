<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter, type LocationQuery } from 'vue-router'

import FailureFilterBar from '@/components/FailureFilterBar.vue'
import FailureList from '@/components/FailureList.vue'
import { useFailureList } from '@/composables/useFailureList'
import { useInstances } from '@/composables/useInstances'
import {
  parseFilterQuery,
  rangeLabel,
  resolveBracket,
  toRouteQuery,
  validateBracket,
  validateDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

/**
 * The backend clamps `limit` to 1..200 (`src/api/failures.rs:84`), so 200 rows
 * is both what the page asks for and the most it can ever be given: the cap
 * notice is detected from the length of the answer, not from the request.
 */
const PAGE_LIMIT = 200

const route = useRoute()
const router = useRouter()

// `route.query` is a `LocationQuery` whose array members may hold nulls;
// `parseFilterQuery` reads the first string either way, so the shapes agree
// at runtime and only need a widen-free cast to type-check.
type FilterQuery = Parameters<typeof parseFilterQuery>[0]

// The URL is the only state source (§6.5): the draft is derived, writes go back
// to the router, and the two watchers compare serialized forms so they cannot
// ping-pong each other into a loop.
const draft = ref<FilterDraft>(parseFilterQuery(route.query as FilterQuery))
const draftQuery = computed(() => JSON.stringify(toRouteQuery(draft.value)))
const routeQuery = computed(() =>
  JSON.stringify(toRouteQuery(parseFilterQuery(route.query as FilterQuery))),
)

const bracket = computed(() => resolveBracket(draft.value, Date.now()))
const invalid = computed(
  () => validateDraft(draft.value) ?? validateBracket(bracket.value, Date.now()),
)
const { groups, markers, error, uncovered } = useFailureList(draft, PAGE_LIMIT)

// The host options come from the instance view, not from the rows currently on
// screen: an operator narrowing to a quiet node must still be able to pick it.
const { instances } = useInstances()
const hosts = computed(() =>
  instances.value.map((instance) => instance.host).filter((host) => host !== ''),
)

const capped = computed(() => !invalid.value && groups.value.length >= PAGE_LIMIT)

function onSelect(traceId: string) {
  void router.push({
    name: 'failure-detail',
    params: { traceId },
    query: toRouteQuery(draft.value),
  })
}

// The count is bracket-scoped and `/uncovered` drops only the host equality, so
// the action clears the host and nothing else: the rows it just counted are
// exactly the rows this bracket shows.
function showUnlabeled() {
  draft.value = { ...draft.value, instance: '' }
}

watch(draftQuery, (serialized) => {
  if (serialized === routeQuery.value) return
  void router.replace({
    name: 'failures-list',
    query: JSON.parse(serialized) as LocationQuery,
  })
})

watch(routeQuery, (serialized) => {
  if (serialized === draftQuery.value) return
  draft.value = parseFilterQuery(JSON.parse(serialized) as FilterQuery)
})
</script>

<template>
  <div class="flex flex-col gap-4 px-6 py-6">
    <FailureFilterBar v-model="draft" :instances="hosts" />

    <p
      v-if="invalid && route.name !== 'failure-detail'"
      data-testid="invalid-range"
      class="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error"
    >
      {{ invalid }}
    </p>
    <!-- On the detail route the drawer owns the refusal, so the list's own
         copy of the same message stays away. -->
    <p
      v-else-if="error && route.name !== 'failure-detail'"
      class="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-sm"
    >
      {{ error }}
    </p>

    <p
      v-if="draft.instance && uncovered > 0 && !invalid"
      data-testid="uncovered-notice"
      role="status"
      class="flex flex-wrap items-center gap-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-sm"
    >
      有 {{ uncovered }} 条遥测没有主机标签，未被此过滤覆盖。
      <button
        class="focus-ring rounded-md bg-surface-2 px-2 py-0.5 text-xs text-accent"
        @click="showUnlabeled"
      >
        查看全部
      </button>
    </p>

    <p v-if="capped" data-testid="cap-notice" role="status" class="text-xs text-muted">
      仅显示前 {{ PAGE_LIMIT }} 组，请收窄时间或筛选。
    </p>

    <FailureList
      :groups="groups"
      :markers="markers"
      :limit="PAGE_LIMIT"
      :dense="false"
      :see-all="false"
      empty-text="当前区间没有失败记录。"
      title="失败列表"
      :subtitle="rangeLabel(draft, Date.now())"
      @select="(group) => group.traceId && onSelect(group.traceId)"
    />

    <!-- The drawer is this page's child route, so the list stays mounted behind it. -->
    <RouterView />
  </div>
</template>
