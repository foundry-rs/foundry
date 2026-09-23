// Bounded, unauthenticated probes. Never print addresses, headers, bodies,
// environment variables, file names from lsof, or process arguments.
const dns = require('node:dns');
const https = require('node:https');
const { execFile } = require('node:child_process');

const hosts = [
  'results-receiver.actions.githubusercontent.com',
  'broker.actions.githubusercontent.com',
  'github.com',
];
function record(kind, fields) {
  process.stdout.write(`${JSON.stringify({ time: new Date().toISOString(), kind, ...fields })}\n`);
}
function errorCode(error) {
  // Do not serialize error messages, stacks, or request objects.
  return String(error?.code ?? 'ERROR').replace(/[^A-Z0-9_]/gi, '').slice(0, 40);
}
function command(file, args) {
  return new Promise((resolve) => {
    execFile(file, args, { timeout: 3000, killSignal: 'SIGKILL', maxBuffer: 8 * 1024 * 1024 },
      (error, stdout) => resolve({ error, stdout }));
  });
}
function lookup(host, kind, operation) {
  return new Promise((resolve) => {
    const start = Date.now();
    let finished = false;
    const done = (error, answers) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      record(kind, { host, ms: Date.now() - start, code: error ? errorCode(error) : 'OK',
        answers: error ? 0 : answers.length });
      resolve();
    };
    const timer = setTimeout(() => done({ code: 'TIMEOUT' }), 5000);
    operation(done);
  });
}
function connect(host) {
  return new Promise((resolve) => {
    const start = Date.now();
    let finished = false;
    let request;
    const done = (error, status) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      record('https', { host, ms: Date.now() - start, code: error ? errorCode(error) : 'OK',
        status: status ?? null });
      request?.destroy();
      resolve();
    };
    const timer = setTimeout(() => done({ code: 'TIMEOUT' }), 8000);
    // A 4xx response still demonstrates DNS + TCP + TLS + HTTP connectivity.
    // No credentials, redirects, response bodies, or artifact API mutations.
    request = https.request({ hostname: host, path: '/', method: 'HEAD', agent: false },
      (response) => done(null, response.statusCode));
    request.on('error', (error) => done(error));
    request.end();
  });
}
async function descriptors() {
  const limits = await command('/usr/sbin/sysctl', ['kern.num_files', 'kern.maxfiles', 'kern.maxfilesperproc']);
  if (limits.error) record('fd-system', { code: errorCode(limits.error) });
  else {
    const values = {};
    for (const line of limits.stdout.split('\n')) {
      const match = line.match(/^(kern\.(?:num_files|maxfiles|maxfilesperproc)):\s*(\d+)$/);
      if (match) values[match[1]] = Number(match[2]);
    }
    record('fd-system', values);
  }
  const processes = await command('/bin/ps', ['-axo', 'pid=,comm=']);
  if (processes.error) return record('fd-processes', { code: errorCode(processes.error) });
  // Bound lsof work even if unexpected processes accumulate on the runner.
  const selected = processes.stdout.split('\n').map((line) => line.trim().match(/^(\d+)\s+(.+)$/))
    .filter((match) => match && /\/(?:Runner\.(?:Worker|Listener)|rustc|cargo|node|HardenRunner|aegis|io\.stepsecurity\.harden-runner\.ext)$/.test(match[2]))
    .slice(0, 16);
  await Promise.all(selected.map(async ([, pid, executable]) => {
    const result = await command('/usr/sbin/lsof', ['-n', '-P', '-a', '-p', pid, '-F', 'f']);
    const count = result.stdout.split('\n').filter((line) => /^f\d+$/.test(line)).length;
    record('fd-process', { pid: Number(pid), process: executable.split('/').pop(),
      count: result.error ? null : count, code: result.error ? errorCode(result.error) : 'OK' });
  }));
}
async function main() {
  record('probe-start', { node: process.version });
  await Promise.all([
    descriptors(),
    ...hosts.flatMap((host) => [
      lookup(host, 'os-lookup', (done) => dns.lookup(host, { all: true }, done)),
      lookup(host, 'dns-a', (done) => dns.resolve4(host, done)),
      connect(host),
    ]),
  ]);
  record('probe-end', {});
}
// The process exits even if a native resolver request cannot be cancelled.
if (require.main === module) {
  const deadline = setTimeout(() => { record('probe-deadline', {}); process.exit(0); }, 20000);
  main().then(() => { clearTimeout(deadline); process.exit(0); },
    (error) => { record('probe-error', { code: errorCode(error) }); process.exit(0); });
}
module.exports = { errorCode, main };
