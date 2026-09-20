const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { MARKER, isBalRequest, authorize, publish } = require('../cast-bal-comment.cjs');

const BASE = 'a'.repeat(40);
const HEAD = 'b'.repeat(40);
const root = path.resolve(__dirname, '../../..');

function mock(options = {}) {
  const outputs = {};
  const calls = [];
  const messages = [];
  const comments = options.comments ?? [];
  const context = {
    repo: { owner: 'foundry-rs', repo: 'foundry' },
    issue: { number: 123 },
    payload: {
      issue: { pull_request: {} },
      comment: {
        id: 456,
        body: 'derek bench bal',
        author_association: 'MEMBER',
        user: { login: 'maintainer' },
      },
    },
    runId: 789,
    serverUrl: 'https://github.com',
  };
  const pr = {
    state: 'open',
    base: { repo: { full_name: 'foundry-rs/foundry' }, sha: BASE },
    head: { repo: { full_name: 'contributor/foundry' }, sha: HEAD },
    ...options.pr,
  };
  const github = {
    rest: {
      repos: { getCollaboratorPermissionLevel: async request => {
        calls.push(['permission', request]);
        return { data: { permission: options.permission ?? 'write' } };
      } },
      pulls: { get: async request => {
        calls.push(['pull', request]);
        return { data: pr };
      } },
      issues: {
        listComments: 'listComments',
        createComment: async request => {
          calls.push(['create', request]);
          comments.push({ id: 42, body: request.body, user: { type: 'Bot', login: 'github-actions[bot]' } });
        },
        updateComment: async request => {
          calls.push(['update', request]);
          comments.find(comment => comment.id === request.comment_id).body = request.body;
        },
      },
    },
    paginate: async (method, request) => {
      calls.push(['list', request]);
      return comments;
    },
  };
  const core = {
    setOutput: (name, value) => { outputs[name] = value; },
    setFailed: message => messages.push(['failed', message]),
    notice: message => messages.push(['notice', message]),
  };
  return { github, context, core, pr, outputs, calls, messages, comments };
}

test('recognizes all aliases without matching other subcommands or embedded prose', () => {
  for (const command of ['derek bench bal', 'decofe bench bal', '@decofe bench bal', 'DEREK bench BAL', 'derek bench bal\n', 'derek bench bal timeout=1']) {
    assert.equal(isBalRequest(command), true, command);
  }
  for (const command of ['derek bench build', 'derek bench ball', 'please derek bench bal', 'derek bench', '', null]) {
    assert.equal(isBalRequest(command), false, command);
  }
});

test('authorizes fork PRs and pins actual base and head, independent of branch names', async () => {
  for (const command of ['derek bench bal', 'decofe bench bal', '@decofe bench bal']) {
    const state = mock();
    state.context.payload.comment.body = command;
    state.pr.base.ref = 'release/next';
    await authorize(state);
    assert.deepEqual(state.outputs, {
      'base-sha': BASE,
      'head-sha': HEAD,
      'head-repository': 'contributor/foundry',
      requested: 'true',
      ready: 'true',
    });
    assert.deepEqual(state.calls[0], ['permission', {
      owner: 'foundry-rs', repo: 'foundry', username: 'maintainer',
    }]);
    assert.equal(state.calls.some(([kind]) => kind === 'create'), false);
  }
});

test('requires both trusted association and current write permission', async () => {
  for (const association of ['NONE', 'CONTRIBUTOR', 'FIRST_TIMER']) {
    const state = mock();
    state.context.payload.comment.author_association = association;
    await authorize(state);
    assert.deepEqual(state.outputs, {});
    assert.equal(state.calls.length, 0);
    assert.equal(state.messages[0][0], 'failed');
  }
  for (const permission of ['read', 'triage', 'none']) {
    const state = mock({ permission });
    await authorize(state);
    assert.deepEqual(state.outputs, {});
    assert.equal(state.calls.length, 1);
    assert.equal(state.messages[0][0], 'failed');
  }
  for (const permission of ['write', 'maintain', 'admin']) {
    const state = mock({ permission });
    await authorize(state);
    assert.equal(state.outputs.ready, 'true');
  }
});

test('ignores issues and unrelated requests, rejects unavailable or malformed refs', async () => {
  const issue = mock();
  delete issue.context.payload.issue.pull_request;
  await authorize(issue);
  assert.equal(issue.calls.length, 0);
  const other = mock();
  other.context.payload.comment.body = 'derek bench invariant';
  await authorize(other);
  assert.equal(other.calls.length, 0);
  for (const override of [
    { state: 'closed' },
    { head: { sha: HEAD, repo: null } },
    { head: { sha: 'main', repo: { full_name: 'fork/repo' } } },
    { base: { sha: BASE, repo: { full_name: 'unrelated/repo' } } },
    { head: { sha: HEAD, repo: { full_name: 'fork/repo\nINJECT=1' } } },
  ]) {
    const state = mock({ pr: override });
    await authorize(state);
    assert.deepEqual(state.outputs, {});
  }
});

test('invalid arguments produce a failure comment request without running builds', async () => {
  const state = mock();
  state.context.payload.comment.body = 'derek bench bal compare-ref=master';
  await authorize(state);
  assert.equal(state.outputs.requested, 'true');
  assert.equal(state.outputs.ready, undefined);
  assert.match(state.outputs['request-error'], /without arguments/);
});

async function withReport(fn) {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'cast-bal-comment-'));
  const filename = path.join(temp, 'report.md');
  fs.writeFileSync(filename, `${MARKER}\n## Cast BAL benchmark\n\nBase → PR data.\n`);
  try {
    await fn({ BASE_SHA: BASE, HEAD_SHA: HEAD, REPORT_PATH: filename, GITHUB_RUN_ATTEMPT: '1' }, temp);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

test('creates one report, updates it on repeated requests and reruns', async () => {
  await withReport(async env => {
    const state = mock();
    await publish({ ...state, env });
    assert.equal(state.comments.length, 1);
    assert.match(state.comments[0].body, /request:456 run:789 attempt:1/);
    state.context.payload.comment.id = 457;
    state.context.runId = 790;
    await publish({ ...state, env });
    env.GITHUB_RUN_ATTEMPT = '2';
    await publish({ ...state, env });
    assert.equal(state.comments.length, 1);
    assert.match(state.comments[0].body, /request:457 run:790 attempt:2/);
    assert.equal(state.calls.filter(([kind]) => kind === 'update').length, 2);
  });
});

test('never replaces a newer request or newer rerun with an older result', async () => {
  await withReport(async env => {
    for (const metadata of ['request:457 run:1 attempt:1', 'request:456 run:790 attempt:1', 'request:456 run:789 attempt:2']) {
      const body = `${MARKER}\n<!-- foundry-cast-bal-${metadata} -->\nnewer result`;
      const state = mock({ comments: [{ id: 1, body, user: { type: 'Bot', login: 'github-actions[bot]' } }] });
      await publish({ ...state, env });
      assert.equal(state.comments[0].body, body);
      assert.equal(state.calls.some(([kind]) => ['create', 'update'].includes(kind)), false);
    }
  });
});

test('only updates the Actions bot marker, ignoring user and unrelated bot text', async () => {
  await withReport(async env => {
    const state = mock({ comments: [
      { id: 1, body: MARKER, user: { type: 'User', login: 'attacker' } },
      { id: 2, body: MARKER, user: { type: 'Bot', login: 'other-app[bot]' } },
      { id: 3, body: `quoted ${MARKER}`, user: { type: 'Bot', login: 'github-actions[bot]' } },
    ] });
    await publish({ ...state, env });
    assert.equal(state.comments.length, 4);
    assert.equal(state.calls.some(([kind]) => kind === 'update'), false);
  });
});

test('marks moved base/head as stale and does not post after the PR closes', async () => {
  await withReport(async env => {
    for (const side of ['base', 'head']) {
      const state = mock();
      state.pr[side].sha = 'c'.repeat(40);
      await publish({ ...state, env });
      assert.match(state.comments[0].body, /Stale revisions/);
    }
    const closed = mock({ pr: { state: 'closed' } });
    await publish({ ...closed, env });
    assert.equal(closed.comments.length, 0);
  });
});

test('missing, oversized, symlinked, or malformed reports become honest failures', async () => {
  await withReport(async (env, temp) => {
    for (const content of [undefined, 'not the renderer output', 'a'.repeat(60001)]) {
      if (content === undefined) fs.rmSync(env.REPORT_PATH);
      else fs.writeFileSync(env.REPORT_PATH, content);
      const state = mock();
      await publish({ ...state, env });
      assert.match(state.comments[0].body, /no performance comparison is available/);
      assert.match(state.comments[0].body, new RegExp(BASE));
      assert.match(state.comments[0].body, /actions\/runs\/789\/attempts\/1/);
    }
    const link = path.join(temp, 'link');
    fs.symlinkSync(env.REPORT_PATH, link);
    const state = mock();
    await publish({ ...state, env: { ...env, REPORT_PATH: link } });
    assert.match(state.comments[0].body, /no performance comparison is available/);
  });
});

test('legacy dispatcher bypasses BAL before fork checks and never dispatches or acknowledges it', async () => {
  const workflow = fs.readFileSync(path.join(root, '.github/workflows/benchmarks-dispatch.yml'), 'utf8');
  const body = workflow.match(/          script: \|\n([\s\S]*?)\n      - name: Acknowledge request/)[1];
  const script = body.split('\n').map(line => line.replace(/^ {12}/, '')).join('\n');
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const validate = new AsyncFunction('github', 'context', 'core', script);
  for (const command of ['derek bench bal', 'decofe bench bal', '@decofe bench bal', 'derek bench bal\nbad=arg']) {
    const state = mock();
    state.context.payload.comment.body = command;
    await validate(state.github, state.context, state.core);
    assert.equal(state.outputs.bal, 'true');
    assert.equal(state.calls.length, 0);
  }
  for (const subcommand of ['invariant', 'symex', 'test', 'build', 'fuzz', 'coverage', 'all']) {
    const state = mock();
    state.pr.head.repo.full_name = 'foundry-rs/foundry';
    state.context.payload.comment.body = `derek bench ${subcommand}`;
    await validate(state.github, state.context, state.core);
    const payload = JSON.parse(Buffer.from(state.outputs['payload-b64'], 'base64'));
    assert.equal(payload.event, subcommand === 'invariant' ? 'scfuzzbench' : 'foundry-bench');
    assert.equal(payload.data.foundry_git_ref, HEAD);
    assert.equal(state.outputs.bal, undefined);
  }
  assert.match(workflow, /- name: Acknowledge request\n\s+if: steps.request.outputs.bal != 'true'/);
  assert.match(workflow, /- name: Publish event\n\s+if: steps.request.outputs.bal != 'true'/);
});

test('workflow keeps trusted code, exact refs, and privileged publication separate', () => {
  const workflow = fs.readFileSync(path.join(root, '.github/workflows/benchmarks-bal.yml'), 'utf8');
  const execution = workflow.slice(0, workflow.indexOf('\n  publish:'));
  const publication = workflow.slice(workflow.indexOf('\n  publish:'));
  assert.equal(/secrets\.|id-token:|: write|persist-credentials: true/.test(execution), false);
  assert.equal(/cargo |--base-cast|--head-cast/.test(publication), false);
  assert.match(publication, /pull-requests: write/);
  assert.match(publication, /if: always\(\) && needs.request.outputs.requested == 'true'/);
  assert.match(workflow, /sha: \$\{\{ needs.request.outputs.base-sha \}\}/);
  assert.match(workflow, /sha: \$\{\{ needs.request.outputs.head-sha \}\}/);
  assert.match(workflow, /repository: \$\{\{ needs.request.outputs.head-repository \}\}/);
  assert.match(workflow, /ref: \$\{\{ matrix.sha \}\}/);
  assert.match(workflow, /cargo "\+\$BUILD_TOOLCHAIN" build --locked --profile profiling --bin cast/);
  assert.match(workflow, /artifact-ids: \$\{\{ needs.tools.outputs.artifact-id \}\}/);
  assert.equal((workflow.match(/ref: \$\{\{ github.sha \}\}/g) ?? []).length, 4);
  assert.equal((workflow.match(/persist-credentials: false/g) ?? []).length, 5);
  assert.match(workflow, /--base-sha "\$BASE_SHA" --head-sha "\$HEAD_SHA"/);
});

test('artifact names permit failed-job retries to reuse earlier successful builds', () => {
  const workflow = fs.readFileSync(path.join(root, '.github/workflows/benchmarks-bal.yml'), 'utf8');
  assert.equal(/name: cast-bal-.*run_attempt/.test(workflow), false);
  assert.equal((workflow.match(/overwrite: true/g) ?? []).length, 3);
  assert.equal((workflow.match(/name: cast-bal-results\n/g) ?? []).length, 2);
  assert.match(workflow, /name: cast-bal-base\n/);
  assert.match(workflow, /name: cast-bal-head\n/);
});
