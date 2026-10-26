<script setup lang="ts">
import { computed } from 'vue'
import VChart from 'vue-echarts'

import type { SeriesPoint } from '@/api/types'
import '@/lib/echarts'

const props = defineProps<{ series: SeriesPoint[] }>()

const option = computed(() => ({
  animationDuration: 220,
  backgroundColor: 'transparent',
  grid: { left: 8, right: 12, top: 28, bottom: 8, containLabel: true },
  tooltip: {
    trigger: 'axis',
    backgroundColor: '#161922',
    borderColor: 'rgba(255,255,255,0.12)',
    textStyle: { color: '#e8eaf0', fontSize: 11 },
  },
  legend: {
    top: 0,
    right: 0,
    textStyle: { color: '#8b90a0', fontSize: 11 },
    itemWidth: 10,
    itemHeight: 2,
  },
  xAxis: {
    type: 'category',
    boundaryGap: false,
    data: props.series.map((point) =>
      new Date(point.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
    ),
    axisLine: { lineStyle: { color: 'rgba(255,255,255,0.08)' } },
    axisLabel: { color: '#8b90a0', fontSize: 10 },
  },
  yAxis: {
    type: 'value',
    splitLine: { lineStyle: { color: 'rgba(255,255,255,0.05)' } },
    axisLabel: { color: '#8b90a0', fontSize: 10 },
  },
  series: [
    {
      name: 'Queries',
      type: 'line',
      smooth: true,
      symbol: 'none',
      lineStyle: { width: 2, color: '#5eead4' },
      areaStyle: { color: 'rgba(94,234,212,0.08)' },
      data: props.series.map((point) => point.queries),
    },
    {
      name: 'Failures',
      type: 'line',
      smooth: true,
      symbol: 'none',
      lineStyle: { width: 2, color: '#fb7185' },
      data: props.series.map((point) => point.failures),
    },
  ],
}))
</script>

<template>
  <div class="panel h-[300px] p-4">
    <VChart class="h-full w-full" :option="option" autoresize />
  </div>
</template>
