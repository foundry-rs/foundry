import assert from 'node:assert/strict';
import test from 'node:test';
import { compare, compose } from '../solc-releases.mjs';

function list(...versions) {
  return { builds: versions.map(version => ({ version, sha256: '0x00' })),
    releases: Object.fromEntries(versions.map(version => [version, version])) };
}

test('semver release boundaries distinguish prereleases', () => {
  assert.ok(compare('0.8.31-pre.1', '0.8.31') < 0);
  assert.equal(compare('0.8.31', '0.8.31'), 0);
  assert.ok(compare('0.8.9', '0.8.31') < 0);
  assert.ok(compare('0.9.0', '0.8.31') > 0);
});

test('preserves svm-rs legacy Linux, ARM, and macOS version ranges', () => {
  const result = compose({
    linux: list('0.8.36'), linuxOld: list('0.4.0'),
    linuxNative: list('0.8.31', '0.8.36'), linuxLegacy: list('0.8.30', '0.8.31-pre.1', '0.8.31'),
    mac: list('0.8.4', '0.8.5', '0.8.24', '0.8.25'), macNative: list('0.8.5', '0.8.24'),
    windows: list('0.8.36'),
  });
  assert.deepEqual(Object.keys(result['linux-amd64'].releases), ['0.4.0', '0.8.36']);
  assert.deepEqual(result['linux-aarch64'].builds.map(b => b.version), ['0.8.30', '0.8.31-pre.1', '0.8.31', '0.8.36']);
  assert.equal(result['linux-aarch64'].builds[1].prerelease, 'pre.1');
  assert.deepEqual(result['macosx-aarch64'].builds.map(b => b.version), ['0.8.4', '0.8.25', '0.8.5', '0.8.24']);
  assert.deepEqual(result['windows-amd64'], list('0.8.36'));
});
