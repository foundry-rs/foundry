const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { EventEmitter } = require('node:events');
const { test } = require('node:test');
const vm = require('node:vm');

test('probes report bounded, allowlisted evidence without sensitive values', async () => {
  const output = [];
  const source = readFileSync(require.resolve('../macos-network-probe.cjs'), 'utf8');
  const requests = [];
  const childCommands = [];
  const modules = {
    'node:dns': {
      lookup: (_host, options, done) => {
        assert.equal(options.all, true);
        done({ code: 'ENOTFOUND', message: 'sensitive-error-detail' });
      },
      resolve4: (_host, done) => done(null, ['sensitive-address']),
    },
    'node:https': {
      request: (options, done) => {
        requests.push(options);
        const request = new EventEmitter();
        request.destroy = () => {};
        request.end = () => queueMicrotask(() => {
          if (options.hostname.startsWith('results-receiver.')) {
            request.emit('error', { code: 'ENOTFOUND', message: 'sensitive-error-detail' });
          } else done({ statusCode: 403 });
        });
        return request;
      },
    },
    'node:child_process': {
      execFile: (file, args, options, done) => {
        childCommands.push({ file, args, options });
        assert.equal(options.timeout, 3000);
        assert.equal(options.killSignal, 'SIGKILL');
        if (file.endsWith('/sysctl')) done(null, 'kern.num_files: 234\nkern.maxfiles: 1000\nkern.maxfilesperproc: 500\n');
        else if (file.endsWith('/ps')) done(null, '123 /some/path/Runner.Worker\n456 /other/private/application\n');
        else if (file.endsWith('/lsof')) done(null, 'p123\nf0\nf1\nf2\nf3\nfcwd\n');
        else assert.fail('Unexpected child command');
      },
    },
  };
  const module = { exports: {} };
  vm.runInNewContext(source, {
    require: (id) => { assert.ok(modules[id]); return modules[id]; },
    module,
    process: { version: 'test', stdout: { write: (line) => output.push(line) } },
    setTimeout, clearTimeout,
  });
  await module.exports.main();
  const records = output.map((line) => JSON.parse(line));
  assert.equal(requests.length, 3);
  for (const request of requests) {
    assert.equal(request.method, 'HEAD');
    assert.equal(request.path, '/');
    assert.equal(request.agent, false);
    assert.equal(request.headers, undefined);
    assert.equal(request.auth, undefined);
  }
  assert.equal(records.filter((r) => r.kind === 'os-lookup' && r.code === 'ENOTFOUND').length, 3);
  assert.equal(records.filter((r) => r.kind === 'https' && r.status === 403 && r.code === 'OK').length, 2);
  assert.equal(records.filter((r) => r.kind === 'https' && r.code === 'ENOTFOUND').length, 1);
  assert.equal(records.find((r) => r.kind === 'fd-process').count, 4);
  assert.equal(childCommands.filter((r) => r.file.endsWith('/lsof')).length, 1);
  assert.equal(records.at(-1).kind, 'probe-end');
  assert.doesNotMatch(output.join(''), /sensitive|private|some\/path/);
});

test('resolver and HTTPS requests terminate when no callback arrives', async () => {
  const output = [];
  const module = { exports: {} };
  const modules = {
    'node:dns': { lookup: () => {}, resolve4: () => {} },
    'node:https': { request: () => {
      const request = new EventEmitter();
      request.end = () => {};
      request.destroy = () => {};
      return request;
    } },
    'node:child_process': { execFile: (_file, _args, _options, done) => done({ code: 'ENOENT' }, '') },
  };
  vm.runInNewContext(readFileSync(require.resolve('../macos-network-probe.cjs'), 'utf8'), {
    require: (id) => modules[id], module,
    process: { version: 'test', stdout: { write: (line) => output.push(line) } },
    setTimeout: (fn) => setTimeout(fn, 5), clearTimeout,
  });
  await module.exports.main();
  const records = output.map((line) => JSON.parse(line));
  assert.equal(records.filter((r) => r.code === 'TIMEOUT').length, 9);
  assert.equal(records.at(-1).kind, 'probe-end');
});
