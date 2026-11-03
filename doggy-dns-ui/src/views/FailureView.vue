<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import FailureDrawer from '@/components/FailureDrawer.vue'
import {
  parseFilterQuery,
  resolveBracket,
  toInvestigationFilter,
  validateBracket,
  validateDraft,
} from '@/lib/failureFilter'

// This view is `failures-list`'s child route, so the list stays mounted behind the
// drawer. Its outer section keeps its own padding rather than the parent's: the
// drawer is a `fixed inset-0` overlay, and the only content that renders in place
// here is the invalid-bracket notice.
const route = useRoute()
const router = useRouter()

// `route.query` is a `LocationQuery` whose array members may hold nulls;
// `parseFilterQuery` reads the first string either way, so the shapes agree
// at runtime and only need a widen-free cast to type-check.
type FilterQuery = Parameters<typeof parseFilterQuery>[0]

const draft = computed(() => parseFilterQuery(route.query as FilterQuery))
// Date.now() per evaluation: a relative deep link resolves at the instant the
// operator opened it, and the bracket never round-trips into the URL.
const bracket = computed(() => resolveBracket(draft.value, Date.now()))
const invalid = computed(
  () => validateDraft(draft.value) ?? validateBracket(bracket.value, Date.now()),
)
const filter = computed(() => toInvestigationFilter(draft.value, Date.now()))
const traceId = computed(() => String(route.params.traceId ?? ''))
const analysisId = computed(() => {
  const value = route.query.analysis
  const raw = typeof value === 'string' ? value.trim() : ''
  return raw !== '' ? raw : undefined
})

function close() {
  // Back to the list, bracket intact: only the analysis deep-link param is
  // dropped, so the rows the operator was looking at come back unchanged.
  // A replace, not a push: the entry being left is the drawer's own, and a push
  // would leave Back pointing at it, reopening the drawer and re-running the
  // investigation that was just closed.
  const rest = { ...route.query }
  delete rest.analysis
  void router.replace({ name: 'failures-list', query: rest })
}
</script>

<template>
  <section class="mx-auto max-w-[1440px] p-5 lg:p-7" aria-label="Failure investigation">
    <!--
      The drawer's refusal has its own id: `invalid-range` stays the page's banner
      alone, so it is the single one on both /failures and /failures/:traceId.
    -->
    <p
      v-if="invalid"
      data-testid="drawer-invalid-range"
      class="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error"
    >
      {{ invalid }}
    </p>
    <FailureDrawer
      v-else-if="traceId"
      :trace-id="traceId"
      :filter="filter"
      :analysis-id="analysisId"
      @close="close"
    />
  </section>
</template>
