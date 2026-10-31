import { effectScope, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError, apiGet, apiPostWithStatus } from '@/api/http'
import type { AnalysisResult, AnalysisSnapshot, EvidenceBundle } from '@/api/types'
import { ANALYSIS_POLL_INTERVAL_MS, useInvestigation } from '@/composables/useInvestigation'

vi.mock('@/api/http', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/api/http')>()
  return { ...actual, apiGet: vi.fn(), apiPostWithStatus: vi.fn() }
})

const TRACE = 'trace 1'
const TRACE_SEGMENT = 'trace%201'

function bundle(fingerprint = 'sha256:a'): EvidenceBundle {
  return {
    traceId: TRACE,
    filter: { windowMinutes: 60, domain: null, responseCode: null },
    generatedAt: 1,
    fingerprint,
    group: {
      key: 'group',
      traceId: TRACE,
      query: 'example.com.',
      qtype: 'A',
      instance: null,
      authority: null,
      responseCode: 'SERVFAIL',
      error: null,
      count: 1,
      firstSeen: 1,
      lastSeen: 2,
      durationMs: null,
    },
    instance: null,
    logs: { items: [], total: 0, truncated: false, omitted: 0 },
    spans: { items: [], total: 0, truncated: false },
    metrics: null,
    missing: [],
  }
}

function result(status: AnalysisResult['status'] = 'ok'): AnalysisResult {
  return {
    status,
    verdict: { category: 'unknown', summary: 'summary', confidence: 'medium' },
    findings: [],
    nextSteps: [],
    notDeterminable: [],
  }
}

function snapshot(overrides: Partial<AnalysisSnapshot> = {}): AnalysisSnapshot {
  return {
    analysisId: 'a1',
    traceId: TRACE,
    status: 'succeeded',
    attempt: 1,
    toolCallCount: 0,
    startedAt: 1,
    finishedAt: 2,
    filter: { windowMinutes: 60, domain: null, responseCode: null },
    evidenceFingerprint: 'sha256:a',
    model: 'model',
    promptVersion: 'v1',
    verdictCategory: 'unknown',
    confidence: 'medium',
    result: result(),
    toolCalls: [],
    error: null,
    ...overrides,
  }
}

function runningSnapshot(): AnalysisSnapshot {
  return snapshot({ status: 'running', result: null, finishedAt: null })
}

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const scopes: Array<{ stop: () => void }> = []

function setup(trace: string | null = TRACE) {
  const traceId = ref<string | null>(trace)
  const windowMinutes = ref(60)
  const scope = effectScope()
  const item = scope.run(() => useInvestigation(traceId, windowMinutes))
  if (!item) throw new Error('composable did not run')
  scopes.push(scope)
  return { traceId, windowMinutes, scope, item }
}

function getPaths(): string[] {
  return vi.mocked(apiGet).mock.calls.map(([path]) => path)
}

function postPaths(): string[] {
  return vi.mocked(apiPostWithStatus).mock.calls.map(([path]) => path)
}

const getMock = () => vi.mocked(apiGet)
const postMock = () => vi.mocked(apiPostWithStatus)

beforeEach(() => {
  vi.useFakeTimers()
  getMock().mockReset()
  postMock().mockReset()
})

afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  vi.useRealTimers()
})

describe('useInvestigation', () => {
  it('loads_evidence_and_reaches_evidence_ready', async () => {
    const { item } = setup()

    expect(item.state.value).toBe('idle')

    getMock().mockResolvedValueOnce(bundle())
    await expect(item.loadEvidence()).resolves.toBe(true)

    const path = getPaths()[0]
    expect(path).toContain(`/api/failures/${TRACE_SEGMENT}/evidence`)
    expect(path).toContain('windowMinutes=60')
    expect(item.state.value).toBe('evidenceReady')
    expect(item.evidence.value?.fingerprint).toBe('sha256:a')
    expect(item.error.value).toBeNull()
  })

  it('starts_a_job_and_reaches_analysis_done', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    const accepted = deferred<{ status: number; body: { analysisId: string } }>()
    postMock().mockReturnValueOnce(accepted.promise)

    const started = item.startAnalysis()
    expect(item.state.value).toBe('analysisQueued')

    getMock().mockResolvedValueOnce(runningSnapshot()).mockResolvedValueOnce(snapshot())
    accepted.resolve({ status: 202, body: { analysisId: 'a1' } })
    await vi.advanceTimersByTimeAsync(0)

    expect(item.state.value).toBe('analysisRunning')

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS)
    expect(item.state.value).toBe('analysisRunning')

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS)
    await started

    expect(item.state.value).toBe('analysisDone')
    expect(item.analysis.value?.analysisId).toBe('a1')

    const polls = getPaths().slice(1)
    expect(polls).toHaveLength(2)
    for (const path of polls) {
      expect(path).toContain('/api/failures/trace%201/analysis?analysis=a1')
    }
  })

  it('accepts_a_cached_snapshot_without_polling', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockResolvedValueOnce({ status: 200, body: snapshot() })
    await item.startAnalysis()

    expect(item.state.value).toBe('analysisDone')
    expect(item.analysis.value?.status).toBe('succeeded')
    expect(getPaths()).toHaveLength(1)

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS * 3)
    expect(getPaths()).toHaveLength(1)
  })

  it('maps_an_unavailable_result_to_analysis_unavailable', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockResolvedValueOnce({
      status: 200,
      body: snapshot({ result: result('unavailable') }),
    })
    await item.startAnalysis()

    expect(item.state.value).toBe('analysisUnavailable')
    expect(item.analysis.value?.result?.status).toBe('unavailable')
  })

  it('maps_failed_and_invalid_jobs_to_analysis_unavailable', async () => {
    const failed = setup()
    getMock().mockResolvedValueOnce(bundle())
    await failed.item.loadEvidence()
    postMock().mockResolvedValueOnce({
      status: 200,
      body: snapshot({ status: 'failed', result: null, error: 'model failed' }),
    })
    await failed.item.startAnalysis()
    expect(failed.item.state.value).toBe('analysisUnavailable')
    expect(failed.item.error.value).toBe('model failed')

    const invalid = setup()
    getMock().mockResolvedValueOnce(bundle())
    await invalid.item.loadEvidence()
    postMock().mockResolvedValueOnce({
      status: 200,
      body: snapshot({ status: 'invalid', result: null, error: 'invalid submission' }),
    })
    await invalid.item.startAnalysis()
    expect(invalid.item.state.value).toBe('analysisUnavailable')
    expect(invalid.item.error.value).toBe('invalid submission')
  })

  it('force_sends_force_and_creates_a_new_attempt', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockResolvedValueOnce({
      status: 200,
      body: snapshot({ attempt: 2 }),
    })
    await item.startAnalysis(true)

    expect(postPaths()[0]).toContain('force=true')
    expect(item.analysis.value?.attempt).toBe(2)
    expect(item.state.value).toBe('analysisDone')
  })

  it('maps_503_to_analysis_unavailable', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockRejectedValueOnce(new ApiError(503, 'not_configured', 'analysis is disabled'))
    await item.startAnalysis()

    expect(item.state.value).toBe('analysisUnavailable')
    expect(item.error.value).toBe('analysis is disabled')
    expect(getPaths()).toHaveLength(1)

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS * 3)
    expect(getPaths()).toHaveLength(1)
  })

  it('surfaces_429_without_leaving_evidence_ready', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockRejectedValueOnce(new ApiError(429, 'rate_limited', 'analysis rate limit'))
    await item.startAnalysis()

    expect(item.state.value).toBe('evidenceReady')
    expect(item.error.value).toBe('analysis rate limit')
    expect(getPaths()).toHaveLength(1)

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS * 3)
    expect(getPaths()).toHaveLength(1)
  })

  it('marks_the_analysis_stale_when_the_evidence_fingerprint_changes', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(bundle('sha256:a'))
    await item.loadEvidence()

    postMock().mockResolvedValueOnce({ status: 200, body: snapshot() })
    await item.startAnalysis()
    expect(item.state.value).toBe('analysisDone')
    expect(item.stale.value).toBe(false)

    getMock().mockResolvedValueOnce(bundle('sha256:b'))
    await item.loadEvidence()

    expect(item.state.value).toBe('stale')
    expect(item.stale.value).toBe(true)

    postMock().mockResolvedValueOnce({
      status: 200,
      body: snapshot({ attempt: 2, evidenceFingerprint: 'sha256:b' }),
    })
    await item.startAnalysis(true)

    expect(item.state.value).toBe('analysisDone')
    expect(item.stale.value).toBe(false)
  })

  it('restores_a_queued_attempt_and_resumes_polling', async () => {
    const { item } = setup()
    getMock().mockResolvedValueOnce(runningSnapshot()).mockResolvedValueOnce(snapshot())

    const restored = item.loadAnalysis('a1')
    await vi.advanceTimersByTimeAsync(0)

    expect(item.state.value).toBe('analysisRunning')
    expect(getPaths()[0]).toContain('/api/failures/trace%201/analysis?analysis=a1')

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS)
    await restored

    expect(item.state.value).toBe('analysisDone')
    expect(item.analysis.value?.analysisId).toBe('a1')
  })

  it('stops_polling_after_scope_dispose', async () => {
    const { item, scope } = setup()
    getMock().mockResolvedValueOnce(bundle())
    await item.loadEvidence()

    postMock().mockResolvedValueOnce({ status: 202, body: { analysisId: 'a1' } })
    await item.startAnalysis()
    expect(item.state.value).toBe('analysisRunning')

    getMock().mockResolvedValue(runningSnapshot())
    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS)
    const callsBeforeDispose = getPaths().length
    expect(callsBeforeDispose).toBe(2)

    scope.stop()
    expect(vi.getTimerCount()).toBe(0)

    await vi.advanceTimersByTimeAsync(ANALYSIS_POLL_INTERVAL_MS * 5)
    expect(getPaths()).toHaveLength(callsBeforeDispose)
  })
})
