#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { isDeepStrictEqual, parseArgs } from 'node:util'
import {
  emitHandler,
  emitSuite,
  FOUNDRY_CONFIG,
  generate,
  MODEL,
  settings,
  uniformAnswers,
  VERSION
} from './grammar.mjs'

const ROOT = dirname(fileURLToPath(import.meta.url))

export async function liveDecision(request, fetcher = fetch) {
  const key = process.env.OPENROUTER_API_KEY?.trim()
  if (!key) throw new Error('OPENROUTER_API_KEY is required for --live')
  const response = await fetcher('https://openrouter.ai/api/alpha/decisions', {
    method: 'POST',
    redirect: 'error',
    signal: AbortSignal.timeout(15000),
    headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' },
    body: JSON.stringify(request)
  })
  if (!response.ok) throw new Error(`Jev returned HTTP ${response.status}; request was not retried`)
  return response.json()
}

export function replayDecisions(recording, config) {
  if (recording?.version !== VERSION || recording.source !== 'live' || !isDeepStrictEqual(recording.settings, config))
    throw new Error('Recording does not match grammar version and generation settings')
  const expected = config.sequences * config.length
  if (!Array.isArray(recording.decisions) || recording.decisions.length !== expected)
    throw new Error('Recording is incomplete')
  let index = 0
  return async request => {
    const entry = recording.decisions[index++]
    if (!entry || !isDeepStrictEqual(entry.request, request))
      throw new Error('Replay context diverged from recorded prefix')
    return entry.response
  }
}

export async function writeProject(output, programs) {
  await mkdir(join(output, 'src'))
  await mkdir(join(output, 'test'))
  const model = await readFile(join(ROOT, 'model.sol'), 'utf8')
  const files = {
    'foundry.toml': FOUNDRY_CONFIG,
    'src/Model.sol': model,
    'src/GeneratedHandler.sol': emitHandler(programs),
    'test/GeneratedInvariant.t.sol': emitSuite(programs)
  }
  const hashes = {}
  for (const [path, content] of Object.entries(files)) {
    await writeFile(join(output, path), content, { flag: 'wx' })
    hashes[path] = createHash('sha256').update(content).digest('hex')
  }
  return hashes
}

export async function main(argv = process.argv.slice(2)) {
  const { values } = parseArgs({
    args: argv,
    options: {
      live: { type: 'boolean', default: false },
      uniform: { type: 'boolean', default: false },
      replay: { type: 'string' },
      output: { type: 'string' },
      seed: { type: 'string', default: '1' },
      sequences: { type: 'string', default: '2' },
      length: { type: 'string', default: '3' },
      exploration: { type: 'string', default: '0.2' },
      help: { type: 'boolean', default: false }
    }
  })
  if (values.help) {
    console.log(
      'node benches/jev/grammar/generate.mjs (--live | --uniform | --replay FILE) --output NEW_DIR [--seed 1 --sequences 2 --length 3 --exploration 0.2]'
    )
    return
  }
  if ([values.live, values.uniform, Boolean(values.replay)].filter(Boolean).length !== 1 || !values.output)
    throw new Error('Choose one of --live, --uniform, --replay and provide --output')
  const config = settings({
    seed: Number(values.seed),
    sequences: Number(values.sequences),
    length: Number(values.length),
    exploration: Number(values.exploration)
  })
  let decide = liveDecision
  if (values.uniform) decide = async () => ({ model: MODEL, answers: uniformAnswers() })
  if (values.replay) decide = replayDecisions(JSON.parse(await readFile(values.replay, 'utf8')), config)
  const output = resolve(values.output)
  await mkdir(output) // Existing output is an error, including after an interrupted run.
  const recording = {
    version: VERSION,
    source: values.live ? 'live' : values.uniform ? 'uniform' : 'replay',
    settings: config,
    decisions: []
  }
  let requestStarted
  const programs = await generate(config, async request => {
    requestStarted = performance.now()
    return decide(request)
  }, async (request, response) => {
    recording.decisions.push({ request, response, durationMs: performance.now() - requestStarted })
    await writeFile(join(output, 'decisions.json'), JSON.stringify(recording, null, 2) + '\n')
  })
  const hashes = await writeProject(output, programs)
  await writeFile(join(output, 'programs.json'), JSON.stringify(programs, null, 2) + '\n')
  await writeFile(
    join(output, 'manifest.json'),
    JSON.stringify(
      {
        version: VERSION,
        settings: config,
        hashes,
        source: recording.source,
        apiCalls: values.live ? recording.decisions.length : 0
      },
      null,
      2
    ) + '\n'
  )
  console.log(`Generated ${programs.length} typed sequences in ${output}`)
  console.log(
    'Change into the output directory, then run forge test to keep corpus and failure caches local to this project.'
  )
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => {
    console.error(error.message)
    process.exitCode = 1
  })
}
