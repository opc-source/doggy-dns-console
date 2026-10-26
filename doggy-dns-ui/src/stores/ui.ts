import { ref } from 'vue'
import { defineStore } from 'pinia'

export const useUiStore = defineStore('ui', () => {
  const drawerTraceId = ref<string | null>(null)

  function openFailure(traceId: string | null) {
    drawerTraceId.value = traceId
  }

  function closeFailure() {
    drawerTraceId.value = null
  }

  return { drawerTraceId, openFailure, closeFailure }
})
