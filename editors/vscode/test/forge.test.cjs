const assert = require("node:assert/strict");
const { execFile } = require("node:child_process");
const { mkdtemp, writeFile, mkdir, rm } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { promisify } = require("node:util");
const { test } = require("node:test");
const { resolveForge, validateForgeLsp, formatterRoot, shouldFormatOnSave, isFormattingVersionCurrent } = require("../out/forge");

async function fixture(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "foundry-editor-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}
async function executable(directory, name, content) {
  const file = path.join(directory, name);
  await writeFile(file, `#!/bin/sh\n${content}\n`, { mode: 0o755 });
  return file;
}

test("PATH resolves only Forge, with or without Solar present", async (t) => {
  const directory = await fixture(t);
  const forge = await executable(directory, "forge", 'test "$*" = "lsp --stdio --help"');
  assert.equal(await resolveForge("forge", directory, { PATH: directory }), forge);
  await validateForgeLsp(forge);
  await executable(directory, "solar", 'exit 99');
  assert.equal(await resolveForge("forge", directory, { PATH: directory }), forge);
  await validateForgeLsp(forge);
});

test("an explicit Forge path wins over PATH and supports spaces", async (t) => {
  const directory = await fixture(t);
  const forge = await executable(directory, "custom forge", 'exit 0');
  await executable(directory, "forge", 'exit 1');
  assert.equal(await resolveForge(forge, directory, { PATH: directory }), forge);
  assert.equal(await resolveForge("./custom forge", directory, { PATH: directory }), forge);
  await validateForgeLsp(forge);
});

test("missing and non-executable Forge report installation and setting help", async (t) => {
  const directory = await fixture(t);
  await assert.rejects(resolveForge("forge", directory, { PATH: directory }), /not found.*getfoundry.sh.*solarLsp.forgePath/);
  await writeFile(path.join(directory, "forge"), "not executable");
  await assert.rejects(resolveForge("forge", directory, { PATH: directory }), /not executable/);
});

test("--version success cannot hide a build without forge lsp", async (t) => {
  const directory = await fixture(t);
  const forge = await executable(directory, "forge", 'test "$1" = "--version"');
  await promisify(execFile)(forge, ["--version"]);
  await assert.rejects(validateForgeLsp(forge), /does not support.*forge lsp --stdio.*upgrade Foundry/);
});

test("legacy formatting uses the nearest project config, with a workspace fallback", async (t) => {
  const directory = await fixture(t);
  const nested = path.join(directory, "nested");
  await mkdir(path.join(nested, "src"), { recursive: true });
  await writeFile(path.join(directory, "foundry.toml"), "[fmt]\ntab_width = 2\n");
  await writeFile(path.join(nested, "foundry.toml"), "[fmt]\ntab_width = 8\n");
  assert.equal(await formatterRoot(path.join(nested, "src", "C.sol"), directory), nested);
  await rm(path.join(nested, "foundry.toml"));
  assert.equal(await formatterRoot(path.join(nested, "src", "C.sol"), directory), directory);
  await rm(path.join(directory, "foundry.toml"));
  assert.equal(await formatterRoot(path.join(nested, "src", "C.sol"), directory), directory);
});

test("legacy save formatting yields when editor.formatOnSave is enabled", () => {
  assert.equal(shouldFormatOnSave(true, true, false), true);
  assert.equal(shouldFormatOnSave(true, true, true), false);
  assert.equal(shouldFormatOnSave(false, true, false), false);
  assert.equal(shouldFormatOnSave(true, false, false), false);
});

test("formatting results are rejected after a document edit or close", () => {
  assert.equal(isFormattingVersionCurrent(3, 3, false), true);
  assert.equal(isFormattingVersionCurrent(3, 4, false), false);
  assert.equal(isFormattingVersionCurrent(3, 3, true), false);
});
