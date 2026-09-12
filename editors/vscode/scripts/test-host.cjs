const { runTests } = require("@vscode/test-electron");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { access, copyFile, link, mkdtemp, mkdir, readFile, writeFile, realpath, symlink } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

async function main() {
  const launcher = process.argv.includes("--launcher");
  const sourceExtensionPath = path.resolve(__dirname, "..");
  let extensionPath = sourceExtensionPath;
  let forgePath = await realpath(process.env.FORGE_PATH || path.resolve(sourceExtensionPath, "../../target/debug/forge"));
  // macOS places VS Code's Unix socket under user-data; its path must fit 103 bytes.
  const testRoot = await realpath(await mkdtemp(launcher && process.platform === "darwin"
    ? "/tmp/fl-"
    : path.join(os.tmpdir(), "foundry-vscode-host-")));
  const workspace = path.join(testRoot, "workspace");
  const nested = path.join(workspace, "nested");
  const binaryDirectory = path.join(testRoot, "bin");
  await mkdir(path.join(workspace, ".vscode"), { recursive: true });
  await mkdir(path.join(nested, "src"), { recursive: true });
  await mkdir(binaryDirectory);
  const standaloneForge = path.join(binaryDirectory, process.platform === "win32" ? "forge.exe" : "forge");
  if (launcher) {
    try {
      await link(forgePath, standaloneForge);
    } catch (error) {
      if (error.code !== "EXDEV") throw error;
      await copyFile(forgePath, standaloneForge);
    }
    forgePath = standaloneForge;
  } else {
    await symlink(forgePath, standaloneForge);
  }
  if (process.env.FOUNDRY_EDITOR_TEST_WITH_SOLAR === "1") {
    await writeFile(path.join(binaryDirectory, "solar"), `#!/bin/sh\nprintf called > '${path.join(testRoot, "solar-called")}'\nexit 99\n`, { mode: 0o755 });
  }
  // Forge's default checks still inherit the selected Forge path.
  await writeFile(path.join(workspace, ".vscode/settings.json"), JSON.stringify({
    ...(process.env.FOUNDRY_EDITOR_TEST_PATH_MODE === "1" ? {} : {
      "solarLsp.forgePath": forgePath,
    }),
    "solarLsp.flychecks": [],
    "solarLsp.formatOnSave": true,
    "editor.formatOnSave": false,
    "security.workspace.trust.enabled": false,
  }, null, 2));
  await writeFile(path.join(workspace, "foundry.toml"), '[profile.default]\nsrc = "src"\n[fmt]\ntab_width = 8\n');
  await writeFile(path.join(nested, "foundry.toml"), '[profile.default]\nsrc = "src"\n[fmt]\ntab_width = 2\n');
  await writeFile(path.join(nested, "src", "Counter.sol"), '// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract Counter { uint public value = ; }\n');
  const launcherEnv = {};
  let launchArgs = [workspace, "--new-window", "--user-data-dir", path.join(testRoot, "user-data"), "--extensions-dir", path.join(testRoot, "extensions")];
  if (launcher) {
    const capture = path.join(testRoot, "launcher.json");
    const recorder = path.join(testRoot, "record-code.cjs");
    await writeFile(recorder, `require("node:fs").writeFileSync(${JSON.stringify(capture)}, JSON.stringify({ args: process.argv.slice(2), forge: process.env.FOUNDRY_LSP_FORGE, profile: process.env.FOUNDRY_PROFILE }));\n`);
    const code = path.join(testRoot, process.platform === "win32" ? "code.cmd" : "code");
    const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
    await writeFile(code, process.platform === "win32"
      ? `@"${process.execPath}" "${recorder}" %*\r\n`
      : `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(recorder)} "$@"\n`, { mode: 0o755 });
    const testHome = path.join(testRoot, "home");
    await mkdir(testHome);
    execFileSync(forgePath, ["lsp", "--vscode", "--code-path", code], {
      cwd: workspace,
      env: { ...process.env, HOME: testHome, USERPROFILE: testHome, PATH: binaryDirectory },
      timeout: 30_000,
      stdio: "pipe",
    });
    const captured = JSON.parse(await readFile(capture, "utf8"));
    assert.equal(captured.forge, forgePath, "Launcher must bind the standalone Forge executable");
    const developmentIndex = captured.args.indexOf("--extensionDevelopmentPath");
    assert.notEqual(developmentIndex, -1);
    extensionPath = captured.args[developmentIndex + 1];
    assert.ok(extensionPath.startsWith(testHome + path.sep), "Embedded client must be extracted into the isolated home");
    await access(path.join(extensionPath, "out/extension.js"));
    await assert.rejects(access(path.join(extensionPath, "node_modules")), { code: "ENOENT" });
    launchArgs = [...captured.args];
    launchArgs.splice(developmentIndex, 2);
    Object.assign(launcherEnv, { HOME: testHome, USERPROFILE: testHome, FOUNDRY_LSP_FORGE: captured.forge, FOUNDRY_PROFILE: captured.profile });
  }
  launchArgs.push("--disable-extensions", "--disable-workspace-trust", "--skip-welcome", "--skip-release-notes");
  if (launcher) Object.assign(process.env, launcherEnv);
  const report = path.resolve(process.env.FOUNDRY_EDITOR_TEST_REPORT || path.join(sourceExtensionPath, "bundle", launcher ? "launcher-test.json" : "host-test.json"));
  await mkdir(path.dirname(report), { recursive: true });
  console.log(`Isolated VS Code profile and logs: ${testRoot}`);
  await runTests({
    vscodeExecutablePath: process.env.VSCODE_EXECUTABLE_PATH,
    extensionDevelopmentPath: extensionPath,
    extensionTestsPath: path.join(sourceExtensionPath, "test", "host-suite.cjs"),
    launchArgs,
    extensionTestsEnv: {
      PATH: binaryDirectory,
      FOUNDRY_LSP_FORGE: "",
      ...launcherEnv,
      FOUNDRY_EDITOR_TEST_LAUNCHER: launcher ? "1" : "0",
      FOUNDRY_EDITOR_TEST_FORGE: forgePath,
      FOUNDRY_EDITOR_TEST_EXTENSION: extensionPath,
      FOUNDRY_EDITOR_TEST_REPORT: report,
      FOUNDRY_EDITOR_TEST_ROOT: testRoot,
    },
  });
  console.log(`Host verification report: ${report}`);
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
