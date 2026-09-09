// Source-only Cargo dependency bundles. No package code runs during preparation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const schema = 1;
const inputs = [
  ':(glob)**/Cargo.toml', ':(glob)**/Cargo.lock', 'Cargo.toml', 'Cargo.lock',
  '.cargo', 'cooldown.toml', '.github/scripts/dependencies.mjs',
  '.github/scripts/solc-releases.mjs', '.github/workflows/dependencies.yml',
  '.github/actions/use-dependencies', '.github/actions/setup-build',
];

function git(root, ...args) {
  return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8' });
}

export function identity(root) {
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

export function checksum(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

export function verifyChecksum(file, expected) {
  assert.match(expected, /^[a-f0-9]{64}$/, 'A gate-provided SHA256 is required');
  assert.equal(checksum(file), expected, 'Dependency artifact checksum mismatch');
}

export function verifyIdentity(actual, expected) {
  assert.deepEqual(actual, expected, 'Dependency bundle does not match this checkout and policy');
}

export function sourceConfig(config, vendor) {
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

export function prepare(root, parent, firewall) {
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
  const env = { ...process.env, CARGO_HOME: cargoHome, RUSTC_WRAPPER: '', CARGO_NET_OFFLINE: 'false' };
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

export function pack(bundle, archive) {
  execFileSync('tar', ['-czf', archive, '-C', bundle, '.'], { stdio: 'inherit' });
  return checksum(archive);
}

export function restore(root, archive, expected, parent, platform) {
  const current = identity(root);
  verifyChecksum(archive, expected);
  // The gate creates the archive; its hash is supplied through needs, not taken
  // from the download. Extraction happens only after authenticating these bytes.
  const bundle = mkdtempSync(join(parent, 'approved-dependencies-'));
  execFileSync('tar', ['-xzf', archive, '-C', bundle], { stdio: 'inherit' });
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

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = process.cwd();
  const parent = process.env.RUNNER_TEMP;
  assert.ok(parent, 'RUNNER_TEMP is required');
  switch (process.argv[2]) {
    case 'prepare': {
      const bundle = prepare(root, parent, process.env.SFW);
      // Release-list acquisition is data-only and separate from Socket's package policy.
      const { writeReleases } = await import('./solc-releases.mjs');
      await writeReleases(bundle);
      const archive = join(dirname(bundle), 'dependencies.tar.gz');
      output('sha256', pack(bundle, archive));
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
}
