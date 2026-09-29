import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtemp, readdir, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { liveDecision, replayDecisions, writeProject } from './generate.mjs'
import {
  emitHandler,
  generate,
  MODEL,
  randomSource,
  requestFor,
  sample,
  settings,
  statement,
  uniformAnswers,
  validateAnswers,
  validatePrograms,
  VERSION
} from './grammar.mjs'

function response(choices = {}) {
  const answers = uniformAnswers()
  for (const [symbol, choice] of Object.entries(choices)) {
    answers[symbol].choice = choice
    answers[symbol].probabilities = Object.fromEntries(
      Object.keys(answers[symbol].probabilities).map(key => [key, Number(key === choice)])
    )
  }
  return { model: MODEL, answers }
}

function finalForgeJson(stdout) {
  const lines = stdout.trim().split('\n')
  return JSON.parse(lines.at(-1))
}

test('invalid grammar bounds are rejected before requesting any decisions', async () => {
  for (const config of [{ seed: -1 }, { seed: 2 ** 32 }, { length: 0 }, { sequences: 9 }, { exploration: NaN }])
    await assert.rejects(generate(config, async () => assert.fail('must not request')))
})

test('fixed seed and decisions produce the same typed programs', async () => {
  const config = { seed: 42, sequences: 3, length: 4 }
  const decide = async () => response()
  const first = await generate(config, decide)
  assert.deepEqual(first, await generate(config, decide))
  assert.notDeepEqual(first, await generate({ ...config, seed: 43 }, decide))
})

test('context contains the actual selected prefix and recorded responses can replay it', async () => {
  const config = settings({ exploration: 0 })
  const recording = { version: VERSION, source: 'live', settings: config, decisions: [] }
  const programs = await generate(config, async request => {
    assert.equal(request.state.prefix.length, request.state.step)
    return response({ action: 'deposit', amount: 'balance', address: 'sender', sender: 'two' })
  }, async (request, response) => recording.decisions.push({ request, response }))
  assert.deepEqual(programs, await generate(config, replayDecisions(recording, config)))
  recording.decisions[1].request.state.prefix[0].sender = 3
  await assert.rejects(generate(config, replayDecisions(recording, config)), /diverged/)
})

test('partial and stale recordings are rejected', () => {
  const config = settings()
  assert.throws(
    () => replayDecisions({ version: VERSION, source: 'live', settings: config, decisions: [] }, config),
    /incomplete/
  )
  assert.throws(() => replayDecisions({ version: 0 }, config), /does not match/)
})

test('the checked-in live Jev response reproduces the executed Solidity without credentials', async () => {
  const recording = JSON.parse(await readFile(new URL('./recorded-decisions.json', import.meta.url), 'utf8'))
  const programs = await generate(recording.settings, replayDecisions(recording, recording.settings))
  assert.equal(
    createHash('sha256').update(emitHandler(programs)).digest('hex'),
    '4bb208ad7ed3d4bf8eba5c680ce2eb9a1984a1d38297b7598eb81132bb69bdff'
  )
})

test('malformed distributions and code-like production names are rejected', () => {
  for (const value of [NaN, Infinity, -1, 1.1, '0.5']) {
    const body = response()
    body.answers.amount.probabilities.balance = value
    assert.throws(() => validateAnswers(body), /probabilities/)
  }
  const bad = response()
  bad.answers.sender.choice = 'one); selfdestruct(payable(msg.sender))'
  assert.throws(() => validateAnswers(bad), /choice/)
  delete bad.answers.action
  assert.throws(() => validateAnswers(bad), /each grammar/)
  assert.throws(
    () => statement({ action: 'deposit', amount: 'balance', address: 'sender', sender: '__proto__' }),
    /production/
  )
})

test('exploration preserves a positive chance for a zero-probability production', () => {
  const answer = response({ action: 'deposit' }).answers.action
  assert.equal(sample('action', answer, 0.95, 0), 'deposit')
  assert.equal(sample('action', answer, 0.95, 0.2), 'withdraw')
  assert.throws(() => sample('action', answer, 1, 0.2), /random draw/)
  assert.equal(randomSource(0)(), randomSource(0)())
})

test('AST validation rejects untyped fields, invalid senders and excessive recursion', () => {
  const node = statement({ action: 'deposit', sender: 'one', amount: 'balance', address: 'sender' })
  validatePrograms([[node]])
  for (
    const bad of [{ ...node, sender: 0 }, { ...node, code: 'something' }, {
      ...node,
      amount: { kind: 'balance', address: node }
    }]
  ) {
    assert.throws(() => emitHandler([[bad]]))
  }
  assert.throws(() => emitHandler([Array(9).fill(node)]), /sequence length/)
})

test('view expressions are emitted at each action and fuzz leaves are parameters', () => {
  const nodes = [
    statement({ action: 'deposit', sender: 'one', amount: 'balance', address: 'sender' }),
    statement({ action: 'deposit', sender: 'two', amount: 'balance', address: 'bytes20' }),
    statement({ action: 'withdraw', sender: 'three', amount: 'uint', address: 'sender' })
  ]
  const code = emitHandler([nodes])
  assert.match(code, /uint256\[3\] calldata rawAmounts/)
  assert.match(code, /uint256 amount = ledger.balanceOf\(sender\);/)
  assert.match(code, /ledger.balanceOf\(address\(uint160\(rawAddresses\[1\]\)\)\)/)
  assert.match(code, /uint256 amount = rawAmounts\[2\];/)
})

test('successful provider responses are recorded before schema validation', async () => {
  const saved = []
  await assert.rejects(
    generate({ sequences: 1, length: 1 }, async () => ({ invalid: true }), async (_, body) => saved.push(body)),
    /model/
  )
  assert.deepEqual(saved, [{ invalid: true }])
})

test('the API boundary uses choices and does not retry an unsuccessful response', async () => {
  const old = process.env.OPENROUTER_API_KEY
  process.env.OPENROUTER_API_KEY = 'test-key'
  try {
    let calls = 0
    await assert.rejects(
      liveDecision(requestFor([], 0, 0), async (url, options) => {
        calls++
        assert.equal(url, 'https://openrouter.ai/api/alpha/decisions')
        assert.equal(options.redirect, 'error')
        assert.equal(JSON.parse(options.body).questions.amount.type, 'choice')
        return new Response('', { status: 503 })
      }),
      /HTTP 503/
    )
    assert.equal(calls, 1)
  } finally {
    if (old === undefined) delete process.env.OPENROUTER_API_KEY
    else process.env.OPENROUTER_API_KEY = old
  }
})

test('generated Solidity preserves runtime balances, sender identity, raw leaves and corpus guidance', {
  skip: !process.env.FORGE_BIN
}, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'jev-grammar-test-'))
  const node = (action, amount, address = 'sender', sender = 'one') => statement({ action, amount, address, sender })
  const programs = [[
    node('deposit', 'balance'),
    node('deposit', 'balance'),
    node('withdraw', 'deposited'),
    node('deposit', 'uint')
  ], [node('deposit', 'balance', 'bytes20', 'two')]]
  await writeProject(directory, programs)
  await writeFile(
    join(directory, 'test', 'Semantics.t.sol'),
    `// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;
import {LocalLedger} from "../src/Model.sol";
import {GeneratedHandler} from "../src/GeneratedHandler.sol";
contract SemanticsTest {
    function test_lateReadsAndSenderBinding() public {
        LocalLedger ledger = new LocalLedger();
        GeneratedHandler handler = new GeneratedHandler(ledger);
        uint256[4] memory amounts;
        uint256[4] memory addresses;
        amounts[3] = 42;
        handler.sequence0(amounts, addresses);
        require(handler.successes() == 4 && handler.expectedReverts() == 0, "stale balance or wrong sender");
        require(ledger.balanceOf(address(1)) == 958 && ledger.depositedBalanceOf(address(1)) == 42, "wrong delta");
        require(ledger.balanceOf(address(2)) == 1000, "wrong actor");
        uint256[1] memory otherAmounts;
        uint256[1] memory otherAddresses;
        otherAddresses[0] = (uint256(1) << 200) + 1;
        handler.sequence1(otherAmounts, otherAddresses);
        require(ledger.depositedBalanceOf(address(2)) == 958, "bytes20 leaf ignored");
    }
    function test_expectedRevertKeepsState() public {
        LocalLedger ledger = new LocalLedger();
        GeneratedHandler handler = new GeneratedHandler(ledger);
        handler.depositRaw(0, type(uint256).max);
        require(handler.expectedReverts() == 1 && ledger.balanceOf(address(1)) == 1000, "bad revert handling");
    }
}
`
  )
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) =>
      !key.startsWith('FOUNDRY_') && !key.startsWith('DAPP_') && !/(TOKEN|KEY|SECRET|PASSWORD)/i.test(key)
    )
  )
  const result = spawnSync(process.env.FORGE_BIN, ['test', '--root', directory, '--json', '--threads', '1'], {
    cwd: directory,
    env,
    encoding: 'utf8',
    timeout: 60000,
    maxBuffer: 8 * 1024 * 1024
  })
  await writeFile(join(directory, 'forge-result.json'), result.stdout)
  const details = (() => {
    try {
      return JSON.stringify(
        Object.fromEntries(
          Object.entries(finalForgeJson(result.stdout)).map((
            [suite, value]
          ) => [
            suite,
            Object.fromEntries(
              Object.entries(value.test_results).map((
                [test, outcome]
              ) => [test, { status: outcome.status, reason: outcome.reason }])
            )
          ])
        )
      )
    } catch {
      return result.stderr
    }
  })()
  assert.equal(result.status, 0, details)
  const suites = finalForgeJson(result.stdout)
  const tests = Object.values(suites).flatMap(suite => Object.values(suite.test_results))
  assert.equal(tests.length, 3)
  assert.ok(tests.every(result => result.status === 'Success'))
  const campaign = tests.find(result => result.kind?.Invariant)?.kind.Invariant
  assert.equal(campaign.runs, 64)
  assert.equal(campaign.calls, 1024)
  for (const name of ['depositRaw', 'withdrawRaw', 'sequence0', 'sequence1'])
    assert.ok(Object.entries(campaign.metrics).some(([key, value]) => key.endsWith(`.${name}`) && value.calls > 0))
  const corpusFiles = await readdir(join(directory, 'corpus'), { recursive: true })
  assert.ok(corpusFiles.some(path => /\.json(?:\.gz)?$/.test(path)), 'expected persisted coverage corpus entries')
  const source = await readFile(join(directory, 'src', 'GeneratedHandler.sol'), 'utf8')
  assert.equal(source, emitHandler(programs))
})
