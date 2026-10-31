import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia, type Pinia } from 'pinia'
import { ref, type Ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

import type { AnalysisResult, AnalysisSnapshot, EvidenceBundle } from '@/api/types'
import FailureDrawer from '@/components/FailureDrawer.vue'
import type { InvestigationState } from '@/composables/useInvestigation'
import { useChatStore } from '@/stores/chat'

interface Investigation {
  state: Ref<InvestigationState>
  evidence: Ref<EvidenceBundle | null>
  analysis: Ref<AnalysisSnapshot | null>
  error: Ref<string | null>
  stale: Ref<boolean>
  loadEvidence: Mock<() => Promise<boolean>>
  startAnalysis: Mock<(force?: boolean) => Promise<boolean>>
  loadAnalysis: Mock<(analysisId?: string) => Promise<boolean>>
}

interface InvestigationOverrides {
  state?: InvestigationState
  evidence?: EvidenceBundle | null
  analysis?: AnalysisSnapshot | null
  error?: string | null
  stale?: boolean
  loadEvidence?: () => Promise<boolean>
  startAnalysis?: (force?: boolean) => Promise<boolean>
}

function createInvestigation(overrides: InvestigationOverrides = {}): Investigation {
  return {
    state: ref<InvestigationState>(overrides.state ?? 'idle'),
    evidence: ref<EvidenceBundle | null>(overrides.evidence ?? null),
    analysis: ref<AnalysisSnapshot | null>(overrides.analysis ?? null),
    error: ref<string | null>(overrides.error ?? null),
    stale: ref(overrides.stale ?? false),
    loadEvidence: vi.fn(overrides.loadEvidence ?? (async () => true)),
    startAnalysis: vi.fn(overrides.startAnalysis ?? (async () => true)),
    loadAnalysis: vi.fn(async () => true),
  }
}

// `vi.mock` factories are hoisted above the imports; the factory only reads
// `current` when the component calls `useInvestigation()` during setup.
let current: Investigation

vi.mock('@/composables/useInvestigation', () => ({
  useInvestigation: () => current,
  ANALYSIS_POLL_INTERVAL_MS: 1000,
}))

const AT = Date.UTC(2026, 0, 1, 12, 0, 0)

function bundle(overrides: Partial<EvidenceBundle> = {}): EvidenceBundle {
  return {
    traceId: 't1',
    filter: { windowMinutes: 60, domain: null, responseCode: null },
    generatedAt: AT,
    fingerprint: 'sha256:abc123',
    group: {
      key: 'trace:t1',
      traceId: 't1',
      query: 'a.example.',
      qtype: 'A',
      instance: 'dns-1',
      authority: 'forward',
      responseCode: 'SERVFAIL',
      error: 'timeout',
      count: 2,
      firstSeen: 1,
      lastSeen: 2,
      durationMs: 10,
    },
    logs: {
      items: [
        {
          id: 'log:1',
          ts: AT,
          severity: 'ERROR',
          message: 'upstream forward lookup failed',
          spanId: 's1',
          query: 'a.example.',
          qtype: 'A',
          responseCode: 'SERVFAIL',
          error: 'timeout',
        },
        {
          id: 'log:2',
          ts: AT,
          severity: 'WARN',
          message: 'retrying upstream',
          spanId: null,
          query: null,
          qtype: null,
          responseCode: null,
          error: null,
        },
      ],
      total: 5,
      truncated: true,
      omitted: 3,
    },
    spans: {
      items: [
        { id: 'span:1', spanId: 's1', name: 'dns.error', durationMs: 12.5, status: 'ERROR' },
        { id: 'span:2', spanId: 's2', name: 'dns.forward', durationMs: null, status: 'OK' },
      ],
      total: 2,
      truncated: false,
    },
    instance: {
      matchQuality: 'exact',
      identity: { serviceInstanceId: 'dns-1', host: 'dns-1.local', ip: '10.0.0.1' },
      reachable: true,
      status: 'running',
      pid: 42,
      uptimeSeconds: 3600,
      listenAddr: '0.0.0.0:53',
      plugins: [{ id: 'plugin:cache', order: 1, kind: 'cache', enabled: true, details: { size: 1000 } }],
      config: { id: 'config:main', content: '[plugins]\ncache = true', redactedKeys: ['secret_key'] },
      runtime: null,
      missing: [],
    },
    metrics: {
      range: { start: AT - 60_000, end: AT },
      host: {
        name: 'dns-1',
        queries: 77,
        failures: 8,
        queriesId: 'metric:host:queries',
        failuresId: 'metric:host:failures',
      },
      global: { queries: 1234, failures: 56 },
      queriesId: 'metric:global:queries',
      failuresId: 'metric:global:failures',
    },
    missing: ['logs_truncated'],
    ...overrides,
  }
}

function result(overrides: Partial<AnalysisResult> = {}): AnalysisResult {
  return {
    status: 'ok',
    verdict: {
      category: 'upstream_timeout',
      summary: '上游转发超时导致 SERVFAIL',
      confidence: 'high',
    },
    findings: [
      {
        claim: '上游转发查询超时',
        evidenceIds: ['log:1', 'span:1', 'tool:get_failure_logs'],
        kind: 'observed',
      },
      {
        claim: '转发链路可能拥塞',
        evidenceIds: ['config:main', 'metric:host:failures'],
        kind: 'inferred',
      },
    ],
    nextSteps: [{ action: '检查上游 DNS', reason: '超时', risk: 'low' }],
    notDeterminable: ['上游主机负载'],
    ...overrides,
  }
}

function snapshot(overrides: Partial<AnalysisSnapshot> = {}): AnalysisSnapshot {
  return {
    analysisId: 'an-1',
    traceId: 't1',
    status: 'succeeded',
    attempt: 2,
    toolCallCount: 3,
    startedAt: AT,
    finishedAt: AT + 5000,
    filter: { windowMinutes: 60, domain: null, responseCode: null },
    evidenceFingerprint: 'sha256:abc123',
    model: 'gpt-5',
    promptVersion: 'v3',
    verdictCategory: 'upstream_timeout',
    confidence: 'high',
    result: result(),
    toolCalls: [
      {
        name: 'get_failure_logs',
        canonicalArgs: { limit: 20 },
        durationMs: 120,
        resultHash: 'hash-1',
        summary: '获取 5 条日志',
      },
    ],
    error: null,
    ...overrides,
  }
}

const wrappers: VueWrapper[] = []

function mountDrawer(pinia: Pinia = createPinia()): VueWrapper {
  setActivePinia(pinia)
  const wrapper = mount(FailureDrawer, {
    props: { traceId: 't1', windowMinutes: 60 },
    global: { plugins: [pinia] },
    attachTo: document.body,
  })
  wrappers.push(wrapper)
  return wrapper
}

function refButton(wrapper: VueWrapper, evidenceId: string) {
  const match = wrapper
    .findAll('[data-test="evidence-ref"]')
    .find((button) => button.attributes('data-evidence-id') === evidenceId)
  if (!match) throw new Error(`missing evidence ref ${evidenceId}`)
  return match
}

beforeEach(() => {
  current = createInvestigation()
})

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('FailureDrawer investigation panel', () => {
  it('auto-loads evidence, then starts analysis, on open', async () => {
    const investigation = createInvestigation({ state: 'idle' })
    investigation.loadEvidence.mockImplementation(async () => {
      investigation.evidence.value = bundle()
      investigation.state.value = 'evidenceReady'
      return true
    })
    current = investigation

    const wrapper = mountDrawer()
    await flushPromises()

    expect(investigation.loadEvidence).toHaveBeenCalledTimes(1)
    expect(investigation.startAnalysis).toHaveBeenCalledTimes(1)
    expect(wrapper.find('[data-testid="investigation-panel"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="investigation-state"]').text()).toBe('证据就绪')
  })

  it('reports an evidence failure and does not start analysis', async () => {
    const investigation = createInvestigation({ state: 'idle' })
    investigation.loadEvidence.mockImplementation(async () => {
      investigation.error.value = 'evidence boom'
      investigation.state.value = 'idle'
      return false
    })
    current = investigation

    const wrapper = mountDrawer()
    await flushPromises()

    expect(investigation.startAnalysis).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('evidence boom')
    const reload = wrapper.find('[data-test="reload-evidence"]')
    expect(reload.exists()).toBe(true)

    await reload.trigger('click')
    await flushPromises()

    expect(investigation.loadEvidence).toHaveBeenCalledTimes(2)
    expect(investigation.startAnalysis).not.toHaveBeenCalled()
  })

  it('renders the conclusion card and observed/inferred findings with evidence refs', async () => {
    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle(),
      analysis: snapshot(),
    })

    const wrapper = mountDrawer()
    await flushPromises()

    const card = wrapper.find('[data-testid="verdict-card"]')
    expect(card.exists()).toBe(true)
    expect(card.text()).toContain('upstream_timeout')
    expect(card.text()).toContain('high')
    expect(card.text()).toContain('上游转发超时导致 SERVFAIL')
    expect(card.text()).toContain('结论')
    expect(card.text()).toContain('上游主机负载')

    const observed = wrapper.find('[data-testid="finding-observed"]')
    const inferred = wrapper.find('[data-testid="finding-inferred"]')
    expect(observed.text()).toContain('上游转发查询超时')
    expect(inferred.text()).toContain('转发链路可能拥塞')

    const refs = wrapper.findAll('[data-test="evidence-ref"]')
    expect(refs).toHaveLength(5)
    expect(refs.map((item) => item.attributes('data-evidence-id'))).toEqual([
      'log:1',
      'span:1',
      'tool:get_failure_logs',
      'config:main',
      'metric:host:failures',
    ])
  })

  it('switches tabs and highlights the cited row when an evidence ref is clicked', async () => {
    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle(),
      analysis: snapshot(),
    })

    const wrapper = mountDrawer()
    await flushPromises()

    await refButton(wrapper, 'log:1').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="tab-logs"]').attributes('aria-selected')).toBe('true')
    expect(wrapper.find('article[data-evidence-id="log:1"]').classes()).toContain('is-cited')
    expect(wrapper.text()).toContain('upstream forward lookup failed')

    await refButton(wrapper, 'config:main').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="tab-instance"]').attributes('aria-selected')).toBe('true')
    expect(wrapper.find('pre[data-evidence-id="config:main"]').classes()).toContain('is-cited')

    // `tool:*` citations do not map to a tab, so the active tab stays put.
    await refButton(wrapper, 'tool:get_failure_logs').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="tab-instance"]').attributes('aria-selected')).toBe('true')
  })

  it('renders logs, spans, instance and metrics evidence tabs', async () => {
    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle(),
      analysis: snapshot(),
    })

    let wrapper = mountDrawer()
    await flushPromises()

    await wrapper.find('[data-test="tab-logs"]').trigger('click')
    const logRow = wrapper.find('article[data-evidence-id="log:1"]')
    expect(logRow.text()).toContain('ERROR')
    expect(logRow.text()).toContain('upstream forward lookup failed')
    expect(wrapper.text()).toContain('共 5 条')
    expect(wrapper.text()).toContain('已省略 3 条')
    expect(wrapper.text()).toContain('已截断')

    await wrapper.find('[data-test="tab-spans"]').trigger('click')
    expect(wrapper.text()).toContain('dns.error')
    expect(wrapper.text()).toContain('12.5 ms')
    expect(wrapper.find('article[data-evidence-id="span:2"]').text()).toContain('—')

    await wrapper.find('[data-test="tab-instance"]').trigger('click')
    expect(wrapper.text()).toContain('plugin:cache')
    expect(wrapper.text()).toContain('secret_key')
    expect(wrapper.text()).toContain('[plugins]')

    await wrapper.find('[data-test="tab-metrics"]').trigger('click')
    expect(wrapper.text()).toContain('1234')
    expect(wrapper.text()).toContain('77')

    wrapper.unmount()

    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle({ instance: null, metrics: null }),
      analysis: snapshot(),
    })
    wrapper = mountDrawer()
    await flushPromises()

    await wrapper.find('[data-test="tab-instance"]').trigger('click')
    expect(wrapper.text()).toContain('未匹配到实例')

    await wrapper.find('[data-test="tab-metrics"]').trigger('click')
    expect(wrapper.text()).toContain('指标不可用')
  })

  it('offers a non-forced retry when the analysis is unavailable', async () => {
    const investigation = createInvestigation({
      state: 'analysisUnavailable',
      error: 'analysis is disabled',
    })
    current = investigation

    const wrapper = mountDrawer()
    await flushPromises()

    expect(wrapper.find('[data-testid="investigation-state"]').text()).toBe('不可用')
    expect(wrapper.text()).toContain('analysis is disabled')

    const retry = wrapper.find('[data-test="retry-analysis"]')
    expect(retry.exists()).toBe(true)
    await retry.trigger('click')

    expect(investigation.startAnalysis).toHaveBeenCalledTimes(1)
    expect(investigation.startAnalysis).toHaveBeenCalledWith()
  })

  it('warns about stale evidence and reanalyzes with force', async () => {
    const investigation = createInvestigation({
      state: 'stale',
      stale: true,
      evidence: bundle(),
      analysis: snapshot({ evidenceFingerprint: 'sha256:old' }),
    })
    current = investigation

    const wrapper = mountDrawer()
    await flushPromises()

    expect(wrapper.text()).toContain('证据已变化，需重新分析')
    const reanalyze = wrapper.findAll('[data-test="reanalyze"]')
    expect(reanalyze).toHaveLength(1)

    await reanalyze[0]!.trigger('click')

    expect(investigation.startAnalysis).toHaveBeenCalledTimes(1)
    expect(investigation.startAnalysis).toHaveBeenCalledWith(true)
  })

  it('shows forensics metadata and tool calls', async () => {
    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle(),
      analysis: snapshot(),
    })

    const wrapper = mountDrawer()
    await flushPromises()

    const forensics = wrapper.find('[data-testid="forensics"]')
    expect(forensics.exists()).toBe(true)
    expect(forensics.text()).toContain('取证过程')
    expect(wrapper.find('[data-test="forensics-attempt"]').text()).toBe('2')
    expect(wrapper.find('[data-test="forensics-fingerprint"]').text()).toBe('sha256:abc123')
    expect(wrapper.find('[data-test="forensics-model"]').text()).toBe('gpt-5')
    expect(wrapper.find('[data-test="forensics-prompt-version"]').text()).toBe('v3')
    expect(forensics.text()).toContain('get_failure_logs')
    expect(forensics.text()).toContain('hash-1')
    expect(forensics.text()).toContain('获取 5 条日志')
  })

  it('hands the investigation to chat and closes', async () => {
    const pinia = createPinia()
    setActivePinia(pinia)
    const chat = useChatStore()
    const send = vi.spyOn(chat, 'send').mockResolvedValue(true)

    current = createInvestigation({
      state: 'analysisDone',
      evidence: bundle(),
      analysis: snapshot(),
    })

    const wrapper = mountDrawer(pinia)
    await flushPromises()

    await wrapper.find('[data-test="handoff"]').trigger('click')
    await flushPromises()

    expect(send).toHaveBeenCalledTimes(1)
    const message = String(send.mock.calls[0]?.[0])
    expect(message).toContain('t1')
    expect(message).toContain('sha256:abc123')
    expect(message).toContain('upstream_timeout')
    expect(message).toContain('上游转发超时导致 SERVFAIL')
    expect(message).toContain('an-1')
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('surfaces a failed snapshot without a verdict card', async () => {
    current = createInvestigation({
      state: 'analysisUnavailable',
      error: 'model failed',
      evidence: bundle(),
      analysis: snapshot({ status: 'failed', result: null, error: 'model failed' }),
    })

    const wrapper = mountDrawer()
    await flushPromises()

    expect(wrapper.text()).toContain('model failed')
    expect(wrapper.find('[data-testid="verdict-card"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="retry-analysis"]').exists()).toBe(true)
  })
})
