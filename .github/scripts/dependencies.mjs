// Source-only Cargo dependency bundles. No package code runs during preparation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import {
  appendFileSync,
  closeSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { writeReleases } from './solc-releases.mjs';

const schema = 1;
const scope = process.env.DEPENDENCY_SCOPE || 'workspace';
assert.ok(['workspace', 'bindings'].includes(scope), 'Expected workspace or bindings scope');
const manifest = scope === 'bindings'
  ? 'crates/forge/tests/fixtures/bind-lock/Cargo.toml'
  : 'Cargo.toml';
const inputs = [
  ...(scope === 'bindings'
    ? [
        'crates/forge/tests/fixtures/bind-lock',
        'crates/forge/tests/cli/bind.rs',
        '.github/workflows/test.yml',
      ]
    : [
        '.cargo',
        ':(glob)**/Cargo.toml',
        ':(glob)**/Cargo.lock',
        'Cargo.toml',
        'Cargo.lock',
        '.github/scripts/dependencies.mjs',
        '.github/scripts/solc-releases.mjs',
        '.github/actions/restore-dependencies',
        '.github/actions/setup-build',
        '.github/workflows/socket-dependencies.yml',
        ':(exclude)crates/forge/tests/fixtures/bind-lock',
      ]),
];

function git(root, ...args) {
  return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8' });
}

function identity(root) {
  // Git applies the checkout's line-ending rules on Windows. Hash the canonical
  // index only after checking it and the worktree against HEAD.
  assert.equal(
    git(root, 'status', '--porcelain', '--untracked-files=all', '--', ...inputs),
    '',
    'Dependency inputs must be committed and unchanged',
  );
  assert.ok(existsSync(join(root, dirname(manifest), 'Cargo.lock')), 'Cargo.lock is required');
  return {
    schema,
    scope,
    commit: git(root, 'rev-parse', 'HEAD').trim(),
    inputs: createHash('sha256')
      .update(git(root, 'ls-files', '--stage', '-z', '--', ...inputs))
      .digest('hex'),
  };
}

function checksum(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

function verifyIdentity(actual, expected) {
  assert.deepEqual(actual, expected, 'Dependency bundle does not match this checkout and policy');
}

function portableSourceConfig(config) {
  // Preserve Cargo's own mappings, including each Git source. The directory is
  // relative to the parent of CARGO_HOME, so the bundle works on every runner.
  assert.equal((config.match(/^directory = /gm) || []).length, 1, 'Expected one vendor directory');
  return config.replace(/^directory = .*$/m, 'directory = "cargo-home/vendor"');
}

function output(name, value, file = process.env.GITHUB_OUTPUT) {
  assert.ok(file, 'GitHub output file is required');
  assert.ok(!/[\r\n]/.test(value), 'Output must be a single line');
  appendFileSync(file, `${name}=${value}\n`);
}

async function prepare(root, parent, sfwCargo) {
  assert.ok(sfwCargo, 'Socket Firewall Cargo shim is required');
  const before = identity(root);
  const work = mkdtempSync(join(parent, `dependencies-${scope}-`));
  const bundle = join(work, 'bundle');
  const fetchHome = join(work, 'fetch-home');
  const bundleCargoHome = join(bundle, 'cargo-home');
  mkdirSync(fetchHome, { recursive: true });
  mkdirSync(bundleCargoHome, { recursive: true });

  // An empty Cargo home means restored sources cannot bypass today's policy.
  // Fetch without compiling, then vendor from only that approved cache.
  const env = {
    ...process.env,
    CARGO_HOME: fetchHome,
    CARGO_NET_GIT_FETCH_WITH_CLI: 'true',
    CARGO_NET_OFFLINE: 'false',
    RUSTC_WRAPPER: '',
  };
  execFileSync(sfwCargo, ['fetch', '--locked', '--manifest-path', manifest], {
    cwd: root,
    env,
    stdio: 'inherit',
  });
  const config = execFileSync(
    'cargo',
    [
      'vendor',
      '--frozen',
      '--versioned-dirs',
      '--manifest-path',
      manifest,
      join(bundleCargoHome, 'vendor'),
    ],
    {
      cwd: root,
      env: { ...env, CARGO_NET_OFFLINE: 'true' },
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'inherit'],
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  writeFileSync(join(bundleCargoHome, 'config.toml'), portableSourceConfig(config));
  verifyIdentity(identity(root), before);
  writeFileSync(join(bundle, 'identity.json'), `${JSON.stringify(before)}\n`);
  if (scope === 'workspace') await writeReleases(bundle);

  const archive = join(work, `${scope}-dependencies.tar.gz`);
  tar(bundle, archive, true);
  output('sha256', checksum(archive));
  output('archive', archive);
}

function tar(directory, archive, create) {
  // Stream through a descriptor so tar never interprets Windows drive letters
  // as remote hosts.
  const descriptor = openSync(archive, create ? 'w' : 'r');
  try {
    execFileSync('tar', create ? ['-czf', '-', '.'] : ['-xzf', '-'], {
      cwd: directory,
      stdio: create ? ['ignore', descriptor, 'inherit'] : [descriptor, 'inherit', 'inherit'],
    });
  } finally {
    closeSync(descriptor);
  }
}

function restoreBindings(root, archive, parent) {
  const bundle = mkdtempSync(join(parent, 'approved-binding-dependencies-'));
  tar(bundle, archive, false);
  verifyIdentity(JSON.parse(readFileSync(join(bundle, 'identity.json'), 'utf8')), identity(root));
  appendFileSync(join(bundle, 'cargo-home/config.toml'), '\n[net]\noffline = true\n');
  output('FOUNDRY_BINDINGS_CARGO_HOME', join(bundle, 'cargo-home'), process.env.GITHUB_ENV);
}

function restoreWorkspace(root, archive, expected, platform) {
  const bundle = join(root, '.approved-dependencies');
  assert.ok(!existsSync(bundle), 'Approved dependency directory already exists');
  mkdirSync(bundle);
  tar(bundle, archive, false);
  verifyIdentity(JSON.parse(readFileSync(join(bundle, 'identity.json'), 'utf8')), identity(root));

  const sourceConfig = readFileSync(join(bundle, 'cargo-home/config.toml'), 'utf8').replace(
    /^directory = "cargo-home\/vendor"$/m,
    'directory = ".approved-dependencies/cargo-home/vendor"',
  );
  appendFileSync(join(root, '.cargo/config.toml'), `\n${sourceConfig}\n[net]\noffline = true\n`);
  output('CARGO_NET_OFFLINE', 'true', process.env.GITHUB_ENV);

  assert.match(platform, /^(linux-(amd64|aarch64)|macosx-(amd64|aarch64)|windows-amd64)$/);
  const releases = join(bundle, 'solc', `${platform}.json`);
  assert.ok(existsSync(releases), `Missing solc release metadata for ${platform}`);
  // Cargo runs svm-rs-builds from its vendored package root. Keep this path
  // relative so the same configuration survives Docker and cross mounts.
  output('SVM_RELEASES_LIST_JSON', `../../../solc/${platform}.json`, process.env.GITHUB_ENV);
  output('SVM_TARGET_PLATFORM', platform, process.env.GITHUB_ENV);
}

function restore(root, archive, expected, parent, platform) {
  assert.match(expected, /^[a-f0-9]{64}$/, 'A gate-provided SHA256 is required');
  assert.equal(checksum(archive), expected, 'Dependency artifact checksum mismatch');
  if (scope === 'bindings') restoreBindings(root, archive, parent);
  else restoreWorkspace(root, archive, expected, platform || hostPlatform());
}

function hostPlatform() {
  const os = { linux: 'linux', darwin: 'macosx', win32: 'windows' }[process.platform];
  const arch = { x64: 'amd64', arm64: 'aarch64' }[process.arch];
  assert.ok(os && arch, 'Unsupported dependency consumer platform');
  return `${os}-${arch}`;
}

const root = resolve(process.env.DEPENDENCY_ROOT || process.cwd());
const parent = process.env.RUNNER_TEMP;
assert.ok(parent, 'RUNNER_TEMP is required');
switch (process.argv[2]) {
  case 'prepare':
    await prepare(root, parent, process.env.SFW_CARGO);
    break;
  case 'restore':
    restore(
      root,
      process.env.DEPENDENCY_ARCHIVE,
      process.env.DEPENDENCY_SHA256,
      parent,
      process.env.SVM_PLATFORM,
    );
    break;
  default:
    throw new Error('Expected prepare or restore');
}
