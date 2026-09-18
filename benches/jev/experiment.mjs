#!/usr/bin/env node
// A bounded Jev/Forge experiment over the checked-in synthetic arithmetic fixtures.
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { parseArgs } from 'node:util'

const ROOT = dirname(fileURLToPath(import.meta.url))
const ENDPOINT = 'https://openrouter.ai/api/alpha/decisions'
export const MODEL = 'typesafe/jev-1.13'
export const CASES = [
  { id: 'clamp', index: 0, pivot: 100003, specification: 'Clamp a uint32 to the inclusive ceiling 100003.' },
  { id: 'pages', index: 1, pivot: 65521, specification: 'Round a uint32 item count up to whole pages of 65521 items.' },
  {
    id: 'cursor',
    index: 2,
    pivot: 99991,
    specification: 'Increment a uint32 circular cursor, wrapping modulo capacity 99991.'
  }
]
export const STRATEGIES = {
  broad: 'Sample the full uint32 input range.',
  zero: 'Focus on integers 0 through 8.',
  pivot: 'Focus on the nine integers around the ceiling, page size, or capacity in the specification.',
  maximum: 'Focus on the nine largest uint32 values.'
}
const STRATEGY_IDS = Object.keys(STRATEGIES)

export function requestBody() {
  return {
    model: MODEL,
    state: { cases: CASES.map(({ id, specification }) => ({ id, specification })) },
    questions: Object.fromEntries(CASES.map(({ id }) => [id, {
      type: 'choice',
      instructions:
        `Which input distribution best exercises arithmetic boundary behavior for case ${id}? Choose using its specification.`,
      criteria: STRATEGIES
    }]))
  }
}

export function parseDecisions(response) {
  if (typeof response?.model !== 'string' || !response.model) throw new Error('Missing response model')
  return Object.fromEntries(CASES.map(({ id }) => {
    const answer = response.answers?.[id]
    if (!answer || !STRATEGY_IDS.includes(answer.choice)) throw new Error(`Invalid choice for ${id}`)
    const probabilities = answer.probabilities
    if (!probabilities || Object.keys(probabilities).length !== STRATEGY_IDS.length)
      throw new Error(`Incomplete probabilities for ${id}`)
    const values = STRATEGY_IDS.map(key => probabilities[key])
    if (
      values.some(v => typeof v !== 'number' || !Number.isFinite(v) || v < 0 || v > 1)
      || Math.abs(values.reduce((a, b) => a + b, 0) - 1) > 0.01
    ) {
      throw new Error(`Invalid probabilities for ${id}`)
    }
    if (
      typeof answer.confidence !== 'number' || !Number.isFinite(answer.confidence)
      || answer.confidence < 0 || answer.confidence > 1
    ) { throw new Error(`Invalid confidence for ${id}`) }
    return [id, STRATEGY_IDS.indexOf(answer.choice)]
  }))
}

export async function decide(fetcher = fetch) {
  const key = process.env.OPENROUTER_API_KEY?.trim()
  if (!key) throw new Error('OPENROUTER_API_KEY is required for --live')
  const request = requestBody()
  const started = performance.now()
  // No automatic retries: a response may have incurred a charge even if transport failed.
  const response = await fetcher(ENDPOINT, {
    method: 'POST',
    redirect: 'error',
    signal: AbortSignal.timeout(15000),
    headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' },
    body: JSON.stringify(request)
  })
  if (!response.ok) throw new Error(`Jev returned HTTP ${response.status}`)
  const body = await response.json()
  parseDecisions(body)
  return { schemaVersion: 1, request, response: body, durationMs: performance.now() - started }
}

export function validateRecording(recording) {
  if (recording.schemaVersion !== 1 || JSON.stringify(recording.request) !== JSON.stringify(requestBody()))
    throw new Error('Recording does not match this experiment and model')
  if (!Number.isFinite(recording.durationMs) || recording.durationMs < 0) throw new Error('Invalid decision duration')
  return parseDecisions(recording.response)
}

export function parseForgeResult(stdout, exitCode) {
  const suites = JSON.parse(stdout)
  const tests = Object.values(suites).flatMap(suite => Object.entries(suite.test_results ?? {}))
  if (tests.length !== 1 || tests[0][0] !== 'testFuzz_arithmetic(uint32)')
    throw new Error('Forge did not run exactly the expected fixture test')
  const result = tests[0][1]
  if (result.status === 'Success' && exitCode === 0) return { detected: false, result }
  if (
    result.status === 'Failure' && exitCode === 1
    && /^synthetic (clamp|page-count|cursor) mismatch$/.test(result.reason)
  ) {
    return { detected: true, result }
  }
  throw new Error('Unexpected Forge failure; refusing to count it as a finding')
}

function forgeEnvironment(extra = {}) {
  // Ignore ambient Foundry config and never pass model credentials into Forge.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) =>
      !key.startsWith('FOUNDRY_') && !key.startsWith('DAPP_') && !key.startsWith('JEV_')
      && !/(KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)/i.test(key)
    )
  )
  return { ...env, FOUNDRY_PROFILE: 'default', ...extra }
}

function command(forge, args, env) {
  const result = spawnSync(forge, args, { env, encoding: 'utf8', timeout: 60000, maxBuffer: 8 * 1024 * 1024 })
  if (result.error || result.signal) throw new Error(`Forge could not complete: ${result.error?.code ?? result.signal}`)
  return result
}

export function summarize(rows, decisionMs) {
  return ['broad', 'boundaries', 'jev'].map(mode => {
    const samples = rows.filter(row => row.mode === mode && row.mutant)
    const fixtures = new Set(samples.filter(row => row.detected).map(row => row.case))
    const forgeMs = samples.reduce((sum, row) => sum + row.durationMs, 0)
    return {
      mode,
      detections: samples.filter(row => row.detected).length,
      trials: samples.length,
      distinctSyntheticDefects: fixtures.size,
      forgeMs,
      totalMsIncludingOneDecision: forgeMs + (mode === 'jev' ? decisionMs : 0),
      cleanControlFailures: rows.filter(row => row.mode === mode && !row.mutant && row.detected).length
    }
  })
}

export async function main(argv = process.argv.slice(2)) {
  const { values } = parseArgs({
    args: argv,
    options: {
      live: { type: 'boolean', default: false },
      replay: { type: 'string' },
      forge: { type: 'string', default: 'forge' },
      output: { type: 'string' },
      seeds: { type: 'string', default: '20' },
      runs: { type: 'string', default: '256' },
      help: { type: 'boolean', default: false }
    }
  })
  if (values.help) {
    console.log(
      'node benches/jev/experiment.mjs (--live | --replay recording.json) --output NEW_DIR [--forge PATH] [--seeds 20] [--runs 256]'
    )
    return
  }
  if (values.live === Boolean(values.replay) || !values.output)
    throw new Error('Choose exactly one of --live/--replay and provide --output')
  const seeds = Number(values.seeds)
  const runs = Number(values.runs)
  if (!Number.isInteger(seeds) || seeds < 1 || seeds > 100 || !Number.isInteger(runs) || runs < 1 || runs > 100000)
    throw new Error('Expected seeds in 1..100 and runs in 1..100000')
  const output = resolve(values.output)
  await mkdir(output) // Fail on an existing output directory; never overwrite a previous experiment.
  const project = join(ROOT, 'fixtures')
  const env = forgeEnvironment({ FOUNDRY_OUT: join(output, 'out'), FOUNDRY_CACHE_PATH: join(output, 'cache') })
  const version = command(values.forge, ['--version'], env)
  if (version.status !== 0) throw new Error('Cannot determine Forge version')
  const build = command(values.forge, ['build', '--root', project], env)
  await writeFile(join(output, 'build.log'), build.stdout + build.stderr)
  if (build.status !== 0) throw new Error('Fixture build failed; see build.log')
  const recording = values.live ? await decide() : JSON.parse(await readFile(values.replay, 'utf8'))
  // Persist the first successful paid response before running any benchmark work.
  await writeFile(join(output, 'decisions.json'), JSON.stringify(recording, null, 2) + '\n')
  const choices = validateRecording(recording)
  const fixture = await readFile(join(project, 'test', 'Arithmetic.t.sol'))
  const metadata = {
    schemaVersion: 1,
    model: MODEL,
    forgeVersion: version.stdout.trim(),
    seeds,
    runs,
    decisionSource: values.live ? 'live' : 'replay',
    fixtureSha256: createHash('sha256').update(fixture).digest('hex'),
    cases: CASES,
    choices,
    decisionMs: recording.durationMs
  }
  await writeFile(join(output, 'manifest.json'), JSON.stringify(metadata, null, 2) + '\n')
  const rows = []
  for (let seed = 1; seed <= seeds; seed++) {
    for (const testCase of CASES) {
      // Rotate order to avoid systematically giving one arm a warm process/filesystem.
      const modes = ['broad', 'boundaries', 'jev']
      const order = modes.slice(seed % 3).concat(modes.slice(0, seed % 3))
      for (const mode of order) {
        for (const mutant of [false, true]) {
          const strategy = mode === 'broad' ? 0 : mode === 'boundaries' ? 4 : choices[testCase.id]
          const trial = `${seed}-${testCase.id}-${mode}-${mutant ? 'mutant' : 'clean'}`
          const started = performance.now()
          const execution = command(values.forge, [
            'test',
            '--root',
            project,
            '--json',
            '--match-contract',
            '^ArithmeticTest$',
            '--fuzz-seed',
            `0x${seed.toString(16).padStart(64, '0')}`,
            '--fuzz-runs',
            String(runs),
            '--threads',
            '1'
          ], {
            ...env,
            JEV_STRATEGY: String(strategy),
            JEV_CASE: String(testCase.index),
            JEV_PIVOT: String(testCase.pivot),
            JEV_MUTANT: mutant ? '1' : '0',
            FOUNDRY_FUZZ_FAILURE_PERSIST_DIR: join(output, 'failures', trial)
          })
          const durationMs = performance.now() - started
          await writeFile(join(output, `${trial}.json`), execution.stdout)
          await writeFile(join(output, `${trial}.stderr`), execution.stderr)
          const { detected, result } = parseForgeResult(execution.stdout, execution.status)
          rows.push({
            seed,
            case: testCase.id,
            mode,
            mutant,
            strategy,
            detected,
            durationMs,
            reportedFuzzRuns: result.kind?.Fuzz?.runs,
            firstFailureRun: result.counterexample?.Single?.fuzz_run ?? null
          })
          await writeFile(join(output, 'trials.json'), JSON.stringify(rows, null, 2) + '\n')
          if (!mutant && detected) throw new Error(`Clean control failed: ${trial}`)
        }
      }
    }
    console.error(`Completed seed ${seed}/${seeds}`)
  }
  const summary = summarize(rows, recording.durationMs)
  await writeFile(join(output, 'summary.json'), JSON.stringify(summary, null, 2) + '\n')
  console.log(JSON.stringify(summary, null, 2))
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => {
    console.error(error.message)
    process.exitCode = 1
  })
}
