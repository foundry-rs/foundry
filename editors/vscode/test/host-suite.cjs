const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { readFile, writeFile, realpath, access } = require("node:fs/promises");
const path = require("node:path");
const vscode = require("vscode");

async function eventually(check, description) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`Timed out: ${description}`);
}

async function run() {
  const extensionPath = process.env.FOUNDRY_EDITOR_TEST_EXTENSION;
  const forgePath = process.env.FOUNDRY_EDITOR_TEST_FORGE;
  const extension = vscode.extensions.all.find((candidate) => candidate.extensionPath === extensionPath);
  assert.ok(extension, `Development extension must load from ${extensionPath}`);
  await extension.activate();
  const sourcePath = path.join(vscode.workspace.workspaceFolders[0].uri.fsPath, "nested/src/Counter.sol");
  const uri = vscode.Uri.file(sourcePath);
  const document = await vscode.workspace.openTextDocument(uri);
  const editor = await vscode.window.showTextDocument(document);
  await eventually(() => vscode.languages.getDiagnostics(uri).some((diagnostic) => diagnostic.severity === vscode.DiagnosticSeverity.Error), "Forge syntax diagnostics");

  const source = '// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract Counter{function answer() public pure returns(uint){return 9;}}\n';
  await editor.edit((edit) => edit.replace(new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), source));
  assert.ok(document.isDirty);
  await eventually(async () => {
    const symbols = await vscode.commands.executeCommand("vscode.executeDocumentSymbolProvider", uri);
    return symbols?.some((symbol) => symbol.name === "Counter");
  }, "document symbol navigation for unsaved content");
  const hovers = await eventually(async () => {
    const result = await vscode.commands.executeCommand("vscode.executeHoverProvider", uri, new vscode.Position(2, 29));
    return result?.length ? result : undefined;
  }, "hover information");
  const options = { tabSize: 4, insertSpaces: true };
  const edits = await eventually(async () => {
    const result = await vscode.commands.executeCommand("vscode.executeFormatDocumentProvider", uri, options);
    return result?.length ? result : undefined;
  }, "LSP formatting provider");
  const workspaceEdit = new vscode.WorkspaceEdit();
  workspaceEdit.set(uri, edits);
  assert.ok(await vscode.workspace.applyEdit(workspaceEdit));
  const expected = execFileSync(forgePath, ["fmt", "--raw", "--root", path.dirname(path.dirname(sourcePath)), "-"], { input: source, encoding: "utf8" });
  assert.equal(document.getText(), expected, "LSP formatting must use unsaved content and nested foundry.toml");
  assert.match(expected, /\n {2}function answer/);
  assert.match(expected, /return 9;/);
  assert.match(await readFile(sourcePath, "utf8"), /value = ;/, "formatting must not silently save the file");

  const replaceSource = () => editor.edit((edit) => edit.replace(
    new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), source,
  ));
  // Each entry point starts with genuinely unformatted, unsaved content.
  await replaceSource();
  await vscode.commands.executeCommand("solarLsp.formatDocument");
  assert.equal(document.getText(), expected);
  await replaceSource();
  await document.save();
  assert.equal(await readFile(sourcePath, "utf8"), expected);

  // A language override must suppress the legacy hook so VS Code formats only once.
  await vscode.workspace.getConfiguration().update("[solidity]", {
    "editor.formatOnSave": true,
    "editor.defaultFormatter": extension.id,
  }, vscode.ConfigurationTarget.Workspace);
  let formattingRequests = 0;
  const languageClient = require("vscode-languageclient/node");
  const originalSendRequest = languageClient.LanguageClient.prototype.sendRequest;
  languageClient.LanguageClient.prototype.sendRequest = function (type, ...args) {
    if (type === "textDocument/formatting" || type?.method === "textDocument/formatting") formattingRequests++;
    return originalSendRequest.call(this, type, ...args);
  };
  try {
    await replaceSource();
    await document.save();
    assert.equal(await readFile(sourcePath, "utf8"), expected);
    assert.equal(formattingRequests, 1, "language-scoped formatOnSave must issue exactly one formatting request");
  } finally {
    languageClient.LanguageClient.prototype.sendRequest = originalSendRequest;
  }

  const commands = await vscode.commands.getCommands(true);
  for (const command of ["solar.copySelector", "solar.showReferences", "solar.showTypeHierarchy", "solar.clearCache", "solar.reindex"]) {
    assert.ok(commands.includes(command), `${command} must match server protocol commands`);
  }
  let serverProcesses = [];
  if (process.platform !== "win32") {
    const processes = execFileSync("/bin/ps", ["-axo", "pid=,ppid=,command="], { encoding: "utf8" });
    // The expected process is a direct child of this isolated extension host.
    serverProcesses = processes.split("\n").filter((line) => {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
      return match && Number(match[2]) === process.pid && match[3].includes("forge lsp --stdio");
    });
    assert.equal(serverProcesses.length, 1, `Expected one Forge server child, got ${JSON.stringify(serverProcesses)}`);
    const selectedPath = vscode.workspace.getConfiguration("solarLsp").get("forgePath");
    const actualCommand = serverProcesses[0].trim().replace(/^\d+\s+\d+\s+/, "").replace(/ lsp --stdio$/, "");
    assert.equal(await realpath(actualCommand), await realpath(forgePath), `Unexpected server process for ${selectedPath}`);
  }
  await assert.rejects(access(path.join(process.env.FOUNDRY_EDITOR_TEST_ROOT, "solar-called")), { code: "ENOENT" }, "Standalone Solar must never be executed");
  await writeFile(process.env.FOUNDRY_EDITOR_TEST_REPORT, JSON.stringify({
    extensionPath: extension.extensionPath,
    extensionId: extension.id,
    forgePath,
    forgeVersion: execFileSync(forgePath, ["--version"], { encoding: "utf8" }).trim(),
    vscodeVersion: vscode.version,
    serverProcesses,
    checks: ["development extension path", "initialize", "syntax diagnostics", "unsaved document symbols", "hover", "LSP formatting", "nested foundry.toml tab_width = 2", "legacy formatting command", "legacy format on save", "language-scoped save formats exactly once", "solar.* command compatibility", "actual Forge child process"],
    hoverCount: hovers.length,
    isolatedProfile: process.env.FOUNDRY_EDITOR_TEST_ROOT,
  }, null, 2) + "\n");
  console.log("Foundry VS Code Development Host verification passed.");
}
module.exports = { run };
