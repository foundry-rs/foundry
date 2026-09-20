// The issue_comment workflow loads this module only from its trusted event SHA.
const fs = require('node:fs');

const MARKER = '<!-- foundry-cast-bal-benchmark -->';
const REQUEST = /^(?:derek|@?decofe)\s+bench\s+bal(?:\s|$)/i;
const COMMAND = /^(?:derek|@?decofe)\s+bench\s+bal\s*$/i;
const SHA = /^[0-9a-f]{40}$/;
const REPOSITORY = /^[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+$/;
const TRUSTED = new Set(['OWNER', 'MEMBER', 'COLLABORATOR']);
const WRITERS = new Set(['write', 'maintain', 'admin']);

function isBalRequest(body) {
  return typeof body === 'string' && REQUEST.test(body.trim());
}

async function authorize({ github, context, core }) {
  const { comment, issue } = context.payload;
  if (!issue?.pull_request || !isBalRequest(comment?.body)) return;
  if (!TRUSTED.has(comment.author_association)) {
    core.setFailed('BAL benchmarks require a repository collaborator.');
    return;
  }
  const { data: permission } = await github.rest.repos.getCollaboratorPermissionLevel({
    ...context.repo,
    username: comment.user.login,
  });
  if (!WRITERS.has(permission.permission)) {
    core.setFailed('BAL benchmarks require write, maintain, or admin permission.');
    return;
  }
  const { data: pr } = await github.rest.pulls.get({
    ...context.repo,
    pull_number: context.issue.number,
  });
  if (pr.state !== 'open') {
    core.notice('BAL benchmarks only run on open pull requests.');
    return;
  }
  const repository = `${context.repo.owner}/${context.repo.repo}`;
  if (pr.base.repo?.full_name !== repository || !SHA.test(pr.base.sha)
      || !SHA.test(pr.head.sha) || !REPOSITORY.test(pr.head.repo?.full_name ?? '')) {
    core.setFailed('The pull request has no valid source repository or pinned revisions.');
    return;
  }
  core.setOutput('base-sha', pr.base.sha);
  core.setOutput('head-sha', pr.head.sha);
  core.setOutput('head-repository', pr.head.repo.full_name);
  core.setOutput('requested', 'true');
  if (!COMMAND.test(comment.body.trim())) {
    core.setOutput('request-error', 'Use `derek bench bal` without arguments (aliases: `decofe bench bal`, `@decofe bench bal`).');
    return;
  }
  core.setOutput('ready', 'true');
}

function reportOrder(body) {
  const match = body?.match(/<!-- foundry-cast-bal-request:(\d+) run:(\d+) attempt:(\d+) -->/);
  return match ? match.slice(1).map(BigInt) : [0n, 0n, 0n];
}

function compareOrder(a, b) {
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
  }
  return 0;
}

async function publish({ github, context, core, env = process.env }) {
  const base = env.BASE_SHA;
  const head = env.HEAD_SHA;
  if (!SHA.test(base) || !SHA.test(head)) throw new Error('Missing trusted pinned revisions.');
  const run = String(context.runId);
  const attempt = String(env.GITHUB_RUN_ATTEMPT);
  const request = String(context.payload.comment.id);
  if (![run, attempt, request].every(value => /^[1-9]\d*$/.test(value))) {
    throw new Error('Invalid workflow request identity.');
  }
  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${run}/attempts/${attempt}`;
  const { data: pr } = await github.rest.pulls.get({
    ...context.repo,
    pull_number: context.issue.number,
  });
  if (pr.state !== 'open') {
    core.notice('The pull request closed before publication; skipping the comment.');
    return;
  }
  let report;
  try {
    const stat = fs.lstatSync(env.REPORT_PATH);
    if (!stat.isFile() || stat.size > 60000) throw new Error('Invalid report size or type.');
    report = fs.readFileSync(env.REPORT_PATH, 'utf8');
    if (!report.startsWith(`${MARKER}\n`)) throw new Error('Missing report marker.');
  } catch {
    report = `${MARKER}\n## Cast BAL benchmark\n\nThe build, measurement, or report step failed; no performance comparison is available.\n\nBase: \`${base}\` → PR: \`${head}\`.\n\n[View workflow logs](${runUrl})\n`;
  }
  const warnings = [];
  if (pr.base.sha !== base || pr.head.sha !== head) {
    warnings.push('> **Stale revisions:** the PR base or head changed after this request. This report describes only the pinned commits below; run `derek bench bal` again for the current PR.');
  }
  const metadata = `<!-- foundry-cast-bal-request:${request} run:${run} attempt:${attempt} -->`;
  const body = `${MARKER}\n${metadata}\n${warnings.length ? `${warnings.join('\n')}\n\n` : ''}${report.slice(MARKER.length).trimStart()}`;
  const comments = await github.paginate(github.rest.issues.listComments, {
    ...context.repo,
    issue_number: context.issue.number,
    per_page: 100,
  });
  const existing = comments.filter(comment => comment.user?.type === 'Bot'
    && comment.user.login === 'github-actions[bot]' && comment.body?.startsWith(MARKER));
  const order = [request, run, attempt].map(BigInt);
  if (existing.some(comment => compareOrder(reportOrder(comment.body), order) > 0)) {
    core.notice('A newer BAL request already published its report; skipping this older result.');
    return;
  }
  if (existing.length) {
    const latest = existing.reduce((a, b) => compareOrder(reportOrder(a.body), reportOrder(b.body)) >= 0 ? a : b);
    await github.rest.issues.updateComment({ ...context.repo, comment_id: latest.id, body });
  } else {
    await github.rest.issues.createComment({ ...context.repo, issue_number: context.issue.number, body });
  }
}

module.exports = { MARKER, isBalRequest, authorize, publish };
