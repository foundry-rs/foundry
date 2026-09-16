// Prepare only the disposable development profile; never edit the user's settings.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
  cwd: root,
  encoding: "utf8",
}));
const forge = join(metadata.target_directory, "debug", process.platform === "win32" ? "forge.exe" : "forge");
execFileSync(forge, ["lsp", "--stdio", "--help"], { cwd: root, stdio: "pipe" });
const directory = join(root, "target", "editor-dev");
const user = join(directory, "vscode-user-data", "User");
const project = join(directory, "project");
mkdirSync(user, { recursive: true });
mkdirSync(join(directory, "vscode-extensions"), { recursive: true });
mkdirSync(join(project, "src"), { recursive: true });
writeFileSync(join(user, "settings.json"), JSON.stringify({
  "solarLsp.forgePath": forge,
  "solarLsp.trace.server": "verbose",
  "security.workspace.trust.enabled": false,
  "workbench.startupEditor": "none",
  "extensions.autoUpdate": false,
  "extensions.autoCheckUpdates": false,
  "telemetry.telemetryLevel": "off",
}, null, 2) + "\n");
// Preserve scratch edits between debug sessions.
for (const [name, text] of [
  ["foundry.toml", '[profile.default]\nsrc = "src"\n[fmt]\ntab_width = 2\n'],
  ["src/Example.sol", "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract Example { function value() public pure returns(uint256) { return 42; } }\n"],
]) {
  try {
    writeFileSync(join(project, name), text, { flag: "wx" });
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
  }
}
console.log(`Development extension: ${join(root, "editors", "vscode")}`);
console.log(`Forge: ${forge}`);
console.log(`Isolated profile: ${join(directory, "vscode-user-data")}`);
