import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checksum, identity, pack, prepare, restore, sourceConfig, verifyChecksum, verifyIdentity } from '../dependencies.mjs';

function fixture(t) {
  const temp = mkdtempSync(join(tmpdir(), 'foundry-dependencies-test-'));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const root = join(temp, 'repo');
  mkdirSync(root);
  const git = (...args) => execFileSync('git', ['-C', root, ...args], { stdio: 'pipe' });
  git('init');
  git('config', 'user.name', 'Dependency tests');
  git('config', 'user.email', 'tests@example.invalid');
  git('config', 'core.hooksPath', '.git/hooks');
  git('config', 'commit.gpgSign', 'false');
  writeFileSync(join(root, 'Cargo.toml'), '[package]\nname = "fixture"\nversion = "0.1.0"\n');
  writeFileSync(join(root, 'Cargo.lock'), 'version = 4\n');
  git('add', '.');
  git('commit', '-m', 'test: create fixture');
  return { root, temp, git };
}

test('requires a clean, committed dependency graph, including newly added manifests', t => {
  const { root, git } = fixture(t);
  const initial = identity(root);
  assert.equal(initial.schema, 1);
  for (const file of ['Cargo.lock', 'Cargo.toml', 'new/Cargo.toml', '.cargo/config.toml']) {
    const path = join(root, file);
    if (file.includes('/')) mkdirSync(join(root, file.split('/')[0]), { recursive: true });
    writeFileSync(path, 'changed\n');
    assert.throws(() => identity(root), /committed and unchanged/);
    git('add', '.');
    assert.throws(() => identity(root), /committed and unchanged/);
    git('commit', '-m', `test: change ${file}`);
  }
  assert.notEqual(identity(root).inputs, initial.inputs);
});

test('requires a lockfile', t => {
  const { root, git } = fixture(t);
  git('rm', 'Cargo.lock');
  git('commit', '-m', 'test: remove lock');
  assert.throws(() => identity(root), /Cargo.lock is required/);
});

test('rejects missing, corrupt, and different artifact checksums', t => {
  const { root } = fixture(t);
  const file = join(root, 'Cargo.lock');
  verifyChecksum(file, checksum(file));
  for (const digest of ['', '0'.repeat(64), 'x'.repeat(64)]) {
    assert.throws(() => verifyChecksum(file, digest));
  }
});

test('binds the bundle to the commit, policy schema, and dependency inputs', t => {
  const { root } = fixture(t);
  const expected = identity(root);
  verifyIdentity(expected, expected);
  for (const key of ['schema', 'commit', 'inputs']) {
    assert.throws(() => verifyIdentity({ ...expected, [key]: 'other' }, expected), /does not match/);
  }
});

test('preserves git mappings and makes vendor paths portable', () => {
  const config = '[source."git+https://example.invalid/repo"]\ngit = "https://example.invalid/repo"\nreplace-with = "vendored-sources"\n[source.vendored-sources]\ndirectory = "/old/vendor"\n';
  const replaced = sourceConfig(config, 'C:\\runner\\vendor');
  assert.match(replaced, /directory = "C:\/runner\/vendor"/);
  assert.match(replaced, /git = "https:\/\/example.invalid\/repo"/);
  assert.throws(() => sourceConfig('', '/new'));
  assert.throws(() => sourceConfig(config + 'directory = "second"\n', '/new'));
});

test('a rejected acquisition cannot produce a usable bundle', t => {
  const { root, temp } = fixture(t);
  assert.throws(() => prepare(root, temp, ''), /Firewall executable is required/);
  // A benign stand-in for a nonzero Firewall exit; no downloads or package code.
  writeFileSync(join(root, 'cargo'), 'process.exit(42);\n');
  assert.throws(() => prepare(root, temp, process.execPath));
});

test('restores authenticated sources into a fresh offline home and requires platform metadata', t => {
  const { root, temp, git } = fixture(t);
  const bundle = join(temp, 'bundle');
  mkdirSync(join(bundle, 'solc'), { recursive: true });
  mkdirSync(join(bundle, 'vendor'));
  writeFileSync(join(bundle, 'identity.json'), JSON.stringify(identity(root)));
  writeFileSync(join(bundle, 'cargo-config.toml'), '[source.vendored-sources]\ndirectory = "vendor"\n');
  writeFileSync(join(bundle, 'solc/linux-amd64.json'), '{"builds":[],"releases":{}}');
  const archive = join(temp, 'dependencies.tar.gz');
  const digest = pack(bundle, archive);
  const env = restore(root, archive, digest, temp, 'linux-amd64');
  assert.equal(env.CARGO_NET_OFFLINE, 'true');
  assert.match(readFileSync(join(env.CARGO_HOME, 'config.toml'), 'utf8'), /offline = true/);
  const again = restore(root, archive, digest, temp, 'linux-amd64');
  assert.notEqual(env.CARGO_HOME, again.CARGO_HOME);
  assert.throws(() => restore(root, archive, digest, temp, 'macosx-amd64'), /Missing solc/);
  assert.throws(() => restore(root, archive, digest, temp, '../other'));
  writeFileSync(join(root, 'Cargo.lock'), 'version = 3\n');
  git('add', '.');
  git('commit', '-m', 'test: change lock');
  assert.throws(() => restore(root, archive, digest, temp, 'linux-amd64'), /does not match/);
});
