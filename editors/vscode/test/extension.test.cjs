const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const { runInNewContext } = require("node:vm");

test("only a development host honors the Forge launcher executable", async () => {
  for (const [mode, launcher, expected] of [
    [2, "/launcher/forge", "/launcher/forge"],
    [1, "/launcher/forge", "/workspace/forge"],
    [2, undefined, "/workspace/forge"],
  ]) {
    const commands = [];
    const probes = [];
    const disposables = () => ({ dispose() {} });
    const config = {
      get: (key, fallback) => key === "forgePath" ? "/workspace/forge" : fallback,
      inspect: () => undefined,
    };
    const vscode = {
      ExtensionMode: { Production: 1, Development: 2 },
      workspace: {
        isTrusted: true,
        workspaceFolders: [{ uri: { scheme: "file", fsPath: "/project" } }],
        getConfiguration: () => config,
        onWillSaveTextDocument: disposables,
        onDidChangeConfiguration: disposables,
        onDidGrantWorkspaceTrust: disposables,
      },
      commands: { registerCommand: disposables },
      window: { showErrorMessage: (message) => assert.fail(message) },
    };
    const languageclient = {
      State: { Running: 2 },
      LanguageClient: class {
        state = 2;
        initializeResult = { capabilities: { documentFormattingProvider: true } };
        outputChannel = { appendLine() {} };
        constructor(_id, _name, options) { commands.push({ command: options.command, cwd: options.options?.cwd }); }
        async start() {}
        async dispose() {}
      },
    };
    const exports = {};
    runInNewContext(readFileSync(path.join(__dirname, "../out/extension.js"), "utf8"), {
      exports,
      process: { env: { FOUNDRY_LSP_FORGE: launcher } },
      console: { log() {}, error: (message) => assert.fail(message) },
      require: (name) => {
        if (name === "vscode") return vscode;
        if (name === "vscode-languageclient/node") return languageclient;
        if (name === "./forge") return {
          resolveForge: async (command) => command,
          validateForgeLsp: async (command, cwd) => { probes.push({ command, cwd }); },
        };
        return require(name);
      },
    });
    exports.activate({ extensionMode: mode, subscriptions: [] });
    await exports.deactivate();
    assert.deepEqual(commands, [{ command: expected, cwd: "/project" }]);
    assert.deepEqual(probes, commands);
  }
});

test("an untrusted workspace defers Forge LSP startup until trust is granted", async () => {
  let grantTrust;
  let starts = 0;
  const disposables = () => ({ dispose() {} });
  const config = {
    get: (key, fallback) => key === "forgePath" ? "/workspace/forge" : fallback,
    inspect: () => undefined,
  };
  const vscode = {
    ExtensionMode: { Production: 1, Development: 2 },
    workspace: {
      isTrusted: false,
      getConfiguration: () => config,
      onWillSaveTextDocument: disposables,
      onDidChangeConfiguration: disposables,
      onDidGrantWorkspaceTrust: (callback) => {
        grantTrust = callback;
        return disposables();
      },
    },
    commands: { registerCommand: disposables },
    window: { showErrorMessage: (message) => assert.fail(message) },
  };
  const languageclient = {
    State: { Running: 2 },
    LanguageClient: class {
      state = 2;
      initializeResult = { capabilities: { documentFormattingProvider: true } };
      outputChannel = { appendLine() {} };
      constructor() { starts++; }
      async start() {}
      async dispose() {}
    },
  };
  const exports = {};
  runInNewContext(readFileSync(path.join(__dirname, "../out/extension.js"), "utf8"), {
    exports,
    process: { env: {} },
    console: { log() {}, error: (message) => assert.fail(message) },
    require: (name) => {
      if (name === "vscode") return vscode;
      if (name === "vscode-languageclient/node") return languageclient;
      if (name === "./forge") return { resolveForge: async (command) => command, validateForgeLsp: async () => {} };
      return require(name);
    },
  });

  exports.activate({ extensionMode: 1, subscriptions: [] });
  await exports.deactivate();
  assert.equal(starts, 0);

  vscode.workspace.isTrusted = true;
  grantTrust();
  await exports.deactivate();
  assert.equal(starts, 1);
});
