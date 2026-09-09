// Mirror svm-rs 0.5.27's release-list composition without executing build scripts.
// These are compiler metadata inputs, not packages scanned by Socket.
import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const solc = 'https://raw.githubusercontent.com/ethereum/solc-bin/954cd95001614395d9ee3cef03f4c8f94bb508c4';
const linuxArm = 'https://raw.githubusercontent.com/nikitastupin/solc/2287d4326237172acf91ce42fd7ec18a67b7f512/linux/aarch64/list.json';
const macArm = 'https://raw.githubusercontent.com/alloy-rs/solc-builds/e4b80d33bc4d015b2fc3583e217fbf248b2014e1/macosx/aarch64/list.json';

export function compare(version, other) {
  const [core, pre] = version.split('-', 2);
  const a = core.split('.').map(Number);
  const b = other.split('.').map(Number);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return pre ? -1 : 0;
}

function retain(list, predicate) {
  return {
    builds: list.builds.filter(build => predicate(build.version)),
    releases: Object.fromEntries(Object.entries(list.releases).filter(([version]) => predicate(version))),
  };
}

function merge(...lists) {
  return {
    builds: lists.flatMap(list => list.builds),
    releases: Object.assign({}, ...lists.map(list => list.releases)),
  };
}

export function compose({ linux, linuxNative, linuxLegacy, linuxOld, mac, macNative, windows }) {
  const legacy = retain(linuxLegacy, version => compare(version, '0.8.31') < 0);
  for (const build of legacy.builds) build.prerelease = build.version.split('-', 2)[1] || null;
  return {
    'linux-amd64': merge(linuxOld, linux),
    'linux-aarch64': merge(legacy, retain(linuxNative, version => compare(version, '0.8.31') >= 0)),
    'macosx-amd64': mac,
    'macosx-aarch64': merge(retain(mac, version => compare(version, '0.8.5') < 0 || compare(version, '0.8.24') > 0), macNative),
    'windows-amd64': windows,
  };
}

async function json(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(60_000) });
  assert.ok(response.ok, `Release metadata download failed: ${response.status} ${url}`);
  const list = await response.json();
  assert.ok(Array.isArray(list.builds) && Object.keys(list.releases).length, 'Invalid release metadata');
  return list;
}

export async function writeReleases(bundle) {
  // The versioned vendor path deliberately fails on an svm-rs upgrade: review its
  // platform rules and update this acquisition step together with the dependency.
  const svm = join(bundle, 'vendor', 'svm-rs-0.5.27');
  const linuxOld = JSON.parse(readFileSync(join(svm, 'list/linux-arm64-old.json'), 'utf8'));
  const [linux, linuxNative, linuxLegacy, mac, macNative, windows] = await Promise.all([
    json(`${solc}/linux-amd64/list.json`), json(`${solc}/linux-arm64/list.json`), json(linuxArm),
    json(`${solc}/macosx-amd64/list.json`), json(macArm), json(`${solc}/windows-amd64/list.json`),
  ]);
  const directory = join(bundle, 'solc');
  mkdirSync(directory);
  for (const [platform, list] of Object.entries(compose({ linux, linuxNative, linuxLegacy, linuxOld, mac, macNative, windows }))) {
    writeFileSync(join(directory, `${platform}.json`), `${JSON.stringify(list)}\n`);
  }
}
