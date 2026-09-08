const { runTests } = require("@vscode/test-electron");
const { mkdtemp, mkdir, writeFile, realpath, symlink } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

async function main() {
  const extensionPath = path.resolve(__dirname, "..");
  const forgePath = await realpath(process.env.FORGE_PATH || path.resolve(extensionPath, "../../target/debug/forge"));
  const testRoot = await mkdtemp(path.join(os.tmpdir(), "foundry-vscode-host-"));
  const workspace = path.join(testRoot, "workspace");
  const nested = path.join(workspace, "nested");
  const binaryDirectory = path.join(testRoot, "bin");
  await mkdir(path.join(workspace, ".vscode"), { recursive: true });
  await mkdir(path.join(nested, "src"), { recursive: true });
  await mkdir(binaryDirectory);
  await symlink(forgePath, path.join(binaryDirectory, process.platform === "win32" ? "forge.exe" : "forge"));
  if (process.env.FOUNDRY_EDITOR_TEST_WITH_SOLAR === "1") {
    await writeFile(path.join(binaryDirectory, "solar"), `#!/bin/sh\nprintf called > '${path.join(testRoot, "solar-called")}'\nexit 99\n`, { mode: 0o755 });
  }
  // Forge's default checks still inherit the selected Forge path.
  await writeFile(path.join(workspace, ".vscode/settings.json"), JSON.stringify({
    ...(process.env.FOUNDRY_EDITOR_TEST_PATH_MODE === "1" ? {} : { "solarLsp.forgePath": forgePath }),
    "solarLsp.flychecks": [],
    "solarLsp.formatOnSave": true,
    "editor.formatOnSave": false,
    "security.workspace.trust.enabled": false,
  }, null, 2));
  await writeFile(path.join(workspace, "foundry.toml"), '[profile.default]\nsrc = "src"\n[fmt]\ntab_width = 8\n');
  await writeFile(path.join(nested, "foundry.toml"), '[profile.default]\nsrc = "src"\n[fmt]\ntab_width = 2\n');
  await writeFile(path.join(nested, "src", "Counter.sol"), '// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract Counter { uint public value = ; }\n');
  const report = path.resolve(process.env.FOUNDRY_EDITOR_TEST_REPORT || path.join(extensionPath, "bundle", "host-test.json"));
  await mkdir(path.dirname(report), { recursive: true });
  console.log(`Isolated VS Code profile and logs: ${testRoot}`);
  await runTests({
    vscodeExecutablePath: process.env.VSCODE_EXECUTABLE_PATH,
    extensionDevelopmentPath: extensionPath,
    extensionTestsPath: path.join(extensionPath, "test", "host-suite.cjs"),
    launchArgs: [workspace, "--new-window", "--disable-extensions", "--disable-workspace-trust", "--skip-welcome", "--skip-release-notes", "--user-data-dir", path.join(testRoot, "user-data"), "--extensions-dir", path.join(testRoot, "extensions")],
    extensionTestsEnv: {
      PATH: binaryDirectory,
      FOUNDRY_EDITOR_TEST_FORGE: forgePath,
      FOUNDRY_EDITOR_TEST_EXTENSION: extensionPath,
      FOUNDRY_EDITOR_TEST_REPORT: report,
      FOUNDRY_EDITOR_TEST_ROOT: testRoot,
    },
  });
  console.log(`Host verification report: ${report}`);
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
