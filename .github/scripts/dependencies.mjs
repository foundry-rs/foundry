// Source-only Cargo dependency bundles. No package code runs during preparation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFileSync, closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { writeReleases } from './solc-releases.mjs';

const schema = 1;
const inputs = [
  ':(glob)**/Cargo.toml', ':(glob)**/Cargo.lock', 'Cargo.toml', 'Cargo.lock',
  '.cargo', 'cooldown.toml', '.github/scripts/dependencies.mjs',
  '.github/scripts/solc-releases.mjs', '.github/workflows/dependencies.yml',
  '.github/actions/setup-build',
];

function git(root, ...args) {
  return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8' });
}

function identity(root) {
  // Git applies the checkout's line-ending rules on Windows. Hash the canonical
  // index only AFTER checking it and the worktree against HEAD, including new files.
  assert.equal(git(root, 'status', '--porcelain', '--untracked-files=all', '--', ...inputs), '',
    'Dependency inputs must be committed and unchanged');
  assert.ok(existsSync(join(root, 'Cargo.lock')), 'Cargo.lock is required');
  return {
    schema,
    commit: git(root, 'rev-parse', 'HEAD').trim(),
    inputs: createHash('sha256').update(git(root, 'ls-files', '--stage', '-z', '--', ...inputs)).digest('hex'),
  };
}

function checksum(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

function verifyIdentity(actual, expected) {
  assert.deepEqual(actual, expected, 'Dependency bundle does not match this checkout and policy');
}

function sourceConfig(config, vendor) {
  // Preserve Cargo's own mappings, including each git source. Do not reconstruct
  // them from Cargo.lock or silently fall back to the network.
  assert.equal((config.match(/^directory = /gm) || []).length, 1, 'Expected one vendor directory');
  return config.replace(/^directory = .*$/m, `directory = ${JSON.stringify(vendor.replaceAll('\\', '/'))}`);
}

function output(name, value, file = process.env.GITHUB_OUTPUT) {
  assert.ok(file, 'GitHub output file is required');
  assert.ok(!/[\r\n]/.test(value), 'Output must be a single line');
  appendFileSync(file, `${name}=${value}\n`);
}

function prepare(root, parent, firewall) {
  assert.ok(firewall, 'Socket Firewall executable is required');
  const before = identity(root);
  const work = mkdtempSync(join(parent, 'dependencies-'));
  const bundle = join(work, 'bundle');
  const cargoHome = join(work, 'cargo-home');
  mkdirSync(bundle);
  mkdirSync(cargoHome);
  // Empty CARGO_HOME means a previous cache hit cannot bypass current policy.
  // Fetch all locked target dependencies without compiling them. Vendor only
  // from that fresh, approved cache, outside the proxy and with networking off.
  const env = {
    ...process.env, CARGO_HOME: cargoHome, RUSTC_WRAPPER: '', CARGO_NET_OFFLINE: 'false',
    // Git CLI honors Socket's proxy CA; Cargo's built-in Git client does not.
    CARGO_NET_GIT_FETCH_WITH_CLI: 'true',
  };
  execFileSync(firewall, ['cargo', 'fetch', '--locked'], { cwd: root, env, stdio: 'inherit' });
  const config = execFileSync('cargo', [
    'vendor', '--frozen', '--versioned-dirs', join(bundle, 'vendor'),
  ], {
    cwd: root,
    env: { ...env, CARGO_NET_OFFLINE: 'true' },
    encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 16 * 1024 * 1024,
  });
  writeFileSync(join(bundle, 'cargo-config.toml'), sourceConfig(config, 'vendor'));
  verifyIdentity(identity(root), before);
  writeFileSync(join(bundle, 'identity.json'), `${JSON.stringify(before)}\n`);
  return bundle;
}

function tar(bundle, archive, create) {
  // Stream through a file descriptor so tar never interprets Windows drive
  // letters as remote hosts. Node also handles the native working-directory path.
  const fd = openSync(archive, create ? 'w' : 'r');
  try {
    execFileSync('tar', create ? ['-czf', '-', '.'] : ['-xzf', '-'], {
      cwd: bundle, stdio: create ? ['ignore', fd, 'inherit'] : [fd, 'inherit', 'inherit'],
    });
  } finally {
    closeSync(fd);
  }
}

function restore(root, archive, expected, parent, platform) {
  const current = identity(root);
  assert.match(expected, /^[a-f0-9]{64}$/, 'A gate-provided SHA256 is required');
  assert.equal(checksum(archive), expected, 'Dependency artifact checksum mismatch');
  // The gate creates the archive; its hash is supplied through needs, not taken
  // from the download. Extraction happens only after authenticating these bytes.
  const bundle = mkdtempSync(join(parent, 'approved-dependencies-'));
  tar(bundle, archive, false);
  verifyIdentity(JSON.parse(readFileSync(join(bundle, 'identity.json'), 'utf8')), current);
  assert.match(platform, /^(linux-(amd64|aarch64)|macosx-(amd64|aarch64)|windows-amd64)$/);
  const releases = join(bundle, 'solc', `${platform}.json`);
  assert.ok(existsSync(releases), `Missing solc release metadata for ${platform}`);
  const cargoHome = join(bundle, 'cargo-home');
  mkdirSync(cargoHome);
  writeFileSync(join(cargoHome, 'config.toml'),
    `${sourceConfig(readFileSync(join(bundle, 'cargo-config.toml'), 'utf8'), join(bundle, 'vendor'))}\n[net]\noffline = true\n`);
  return { CARGO_HOME: cargoHome, CARGO_NET_OFFLINE: 'true', SVM_RELEASES_LIST_JSON: releases, SVM_TARGET_PLATFORM: platform };
}

function hostPlatform() {
  const os = { linux: 'linux', darwin: 'macosx', win32: 'windows' }[process.platform];
  const arch = { x64: 'amd64', arm64: 'aarch64' }[process.arch];
  assert.ok(os && arch, 'Unsupported dependency consumer platform');
  return `${os}-${arch}`;
}

const root = process.cwd();
const parent = process.env.RUNNER_TEMP;
assert.ok(parent, 'RUNNER_TEMP is required');
switch (process.argv[2]) {
  case 'prepare': {
    const bundle = prepare(root, parent, process.env.SFW);
    // Release-list acquisition is data-only and separate from Socket's package policy.
    await writeReleases(bundle);
    const archive = join(dirname(bundle), 'dependencies.tar.gz');
    tar(bundle, archive, true);
    output('sha256', checksum(archive));
    output('archive', archive);
    break;
  }
  case 'restore': {
    const env = restore(root, process.env.DEPENDENCY_ARCHIVE, process.env.DEPENDENCY_SHA256,
      parent, process.env.SVM_PLATFORM || hostPlatform());
    for (const [key, value] of Object.entries(env)) output(key, value, process.env.GITHUB_ENV);
    break;
  }
  default:
    throw new Error('Expected prepare or restore');
}
