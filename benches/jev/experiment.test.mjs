import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import {
  CASES,
  decide,
  MODEL,
  parseDecisions,
  parseForgeResult,
  requestBody,
  summarize,
  validateRecording
} from './experiment.mjs'

test('the recorded live provider response replays without credentials', async () => {
  const recording = JSON.parse(await readFile(new URL('./recorded-decision.json', import.meta.url), 'utf8'))
  assert.deepEqual(validateRecording(recording), { clamp: 2, pages: 2, cursor: 2 })
})

function response() {
  return {
    model: MODEL,
    answers: Object.fromEntries(CASES.map(({ id }) => [id, {
      type: 'choice',
      choice: 'pivot',
      confidence: 0.9,
      probabilities: { broad: 0.1, zero: 0, pivot: 0.9, maximum: 0 }
    }]))
  }
}

test('one typed batch contains specifications, not mutant source or failure labels', () => {
  const request = requestBody()
  assert.deepEqual(Object.keys(request.questions), ['clamp', 'pages', 'cursor'])
  assert.deepEqual(Object.keys(request.state), ['cases'])
  for (const item of request.state.cases) assert.deepEqual(Object.keys(item), ['id', 'specification'])
})

test('choices are constrained to fixture strategies', () => {
  assert.deepEqual(parseDecisions(response()), { clamp: 2, pages: 2, cursor: 2 })
  for (const choice of ['run-shell', '', null, 2]) {
    const invalid = response()
    invalid.answers.clamp.choice = choice
    assert.throws(() => parseDecisions(invalid), /Invalid choice/)
  }
})

test('missing, non-finite and malformed probabilities are rejected', () => {
  for (const probability of [-0.1, 1.1, NaN, Infinity, '0.9', null]) {
    const invalid = response()
    invalid.answers.clamp.probabilities.pivot = probability
    assert.throws(() => parseDecisions(invalid), /Invalid probabilities/)
  }
  const missing = response()
  delete missing.answers.pages
  assert.throws(() => parseDecisions(missing), /Invalid choice/)
  const incomplete = response()
  delete incomplete.answers.pages.probabilities.zero
  assert.throws(() => parseDecisions(incomplete), /Incomplete probabilities/)
  const badSum = response()
  badSum.answers.pages.probabilities.broad = 0.7
  assert.throws(() => parseDecisions(badSum), /Invalid probabilities/)
})

test('recordings are tied to the exact model and question version', () => {
  const recording = { schemaVersion: 1, request: requestBody(), response: response(), durationMs: 20 }
  assert.deepEqual(validateRecording(recording), { clamp: 2, pages: 2, cursor: 2 })
  recording.request.model = '~typesafe/jev-latest'
  assert.throws(() => validateRecording(recording), /does not match/)
})

test('API uses decision endpoint and never retries a failed response', async () => {
  const original = process.env.OPENROUTER_API_KEY
  process.env.OPENROUTER_API_KEY = 'test-key'
  try {
    let calls = 0
    await assert.rejects(
      decide(async (url, options) => {
        calls++
        assert.equal(url, 'https://openrouter.ai/api/alpha/decisions')
        assert.equal(options.redirect, 'error')
        assert.equal(options.headers.authorization, 'Bearer test-key')
        assert.deepEqual(JSON.parse(options.body), requestBody())
        return new Response('', { status: 503 })
      }),
      /HTTP 503/
    )
    assert.equal(calls, 1)
    const recording = await decide(async () => Response.json(response()))
    assert.deepEqual(validateRecording(recording), { clamp: 2, pages: 2, cursor: 2 })
  } finally {
    if (original === undefined) delete process.env.OPENROUTER_API_KEY
    else process.env.OPENROUTER_API_KEY = original
  }
})

function forgeOutput(status, reason = null, name = 'testFuzz_arithmetic(uint32)') {
  return JSON.stringify({ 'test/Arithmetic.t.sol:ArithmeticTest': { test_results: { [name]: { status, reason } } } })
}

test('only the expected assertion failures count as detections', () => {
  assert.equal(parseForgeResult(forgeOutput('Success'), 0).detected, false)
  assert.equal(parseForgeResult(forgeOutput('Failure', 'synthetic clamp mismatch'), 1).detected, true)
  assert.throws(() => parseForgeResult('{}', 0), /exactly the expected/)
  assert.throws(() => parseForgeResult(forgeOutput('Failure', 'compiler error'), 1), /Unexpected Forge failure/)
  assert.throws(() => parseForgeResult(forgeOutput('Failure', 'env missing', 'setUp()'), 1), /exactly the expected/)
  assert.throws(() => parseForgeResult(forgeOutput('Success'), 1), /Unexpected Forge failure/)
  assert.throws(() => parseForgeResult(forgeOutput('Skipped'), 0), /Unexpected Forge failure/)
})

test('duplicate detections do not inflate distinct findings and decision time is included', () => {
  const rows = [
    { mode: 'jev', case: 'clamp', mutant: true, detected: true, durationMs: 5 },
    { mode: 'jev', case: 'clamp', mutant: true, detected: true, durationMs: 7 },
    { mode: 'jev', case: 'pages', mutant: true, detected: false, durationMs: 10 },
    { mode: 'jev', case: 'clamp', mutant: false, detected: false, durationMs: 2 }
  ]
  assert.deepEqual(summarize(rows, 50)[2], {
    mode: 'jev',
    detections: 2,
    trials: 3,
    distinctSyntheticDefects: 1,
    forgeMs: 22,
    totalMsIncludingOneDecision: 72,
    cleanControlFailures: 0
  })
})
