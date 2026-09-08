import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ReferencesRequest,
  DocumentFormattingRequest,
  ServerOptions,
  State,
} from "vscode-languageclient/node";
import { spawn } from "node:child_process";
import { formatterRoot, resolveForge, shouldFormatOnSave, validateForgeLsp } from "./forge";

let client: LanguageClient | undefined;
let clientLifecycle: Promise<void> = Promise.resolve();
let activeForgePath: string | undefined;
let launcherForgePath: string | undefined;
let fallbackFormatter: vscode.Disposable | undefined;

const restartSettings = [
  "solarLsp.enable",
  "solarLsp.forgePath",
  "solarLsp.serverPath",
  "solarLsp.flychecks",
  "solarLsp.indexing",
  "solarLsp.codeLens.enable",
  "solarLsp.codeLens.selectors",
  "solarLsp.codeLens.references",
  "solarLsp.codeLens.inheritance",
];

export function activate(context: vscode.ExtensionContext) {
  // Only a development host launched by Forge may override workspace binary settings.
  launcherForgePath = (context.extensionMode === vscode.ExtensionMode.Development ||
    context.extensionMode === vscode.ExtensionMode.Test)
    ? process.env.FOUNDRY_LSP_FORGE
    : undefined;
  // Start the LSP server.
  void restartLanguageServer();

  // Register the format document command.
  const formatCommand = vscode.commands.registerCommand(
    "solarLsp.formatDocument",
    async () => {
      const currentConfig = vscode.workspace.getConfiguration("solarLsp");
      if (!currentConfig.get<boolean>("enable", true)) {
        return;
      }

      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "solidity") {
        return;
      }

      const document = editor.document;
      const version = document.version;
      const edits = await formatDocument(document, version);
      // Formatting is asynchronous. Do not apply edits computed for an older
      // document version after the user has edited the document meanwhile.
      if (document.isClosed || document.version !== version) {
        return;
      }
      // Build and apply the edits synchronously inside `editor.edit`. This
      // gives us one final version check immediately before VS Code commits
      // the edit, without a race between `applyEdit` preparation and commit.
      await editor.edit((editBuilder) => {
        if (document.isClosed || document.version !== version) {
          return;
        }
        for (const edit of edits) {
          editBuilder.replace(edit.range, edit.newText);
        }
      });
    },
  );

  // Preserve the legacy setting, deferring to VS Code when its save formatting is enabled.
  const formatOnSave = vscode.workspace.onWillSaveTextDocument((event) => {
    const currentConfig = vscode.workspace.getConfiguration("solarLsp");
    const editorFormatOnSave = vscode.workspace
      .getConfiguration("editor", event.document)
      .get<boolean>("formatOnSave", false);
    if (
      shouldFormatOnSave(
        currentConfig.get<boolean>("enable", true),
        currentConfig.get<boolean>("formatOnSave", true),
        editorFormatOnSave,
      ) &&
      event.document.languageId === "solidity"
    ) {
      event.waitUntil(formatDocument(event.document, event.document.version));
    }
  });

  const configListener = vscode.workspace.onDidChangeConfiguration((event) => {
    if (restartSettings.some((setting) => event.affectsConfiguration(setting))) {
      void restartLanguageServer();
    }
  });

  const copySelectorCommand = vscode.commands.registerCommand(
    "solar.copySelector",
    copySelector,
  );
  const showReferencesCommand = vscode.commands.registerCommand(
    "solar.showReferences",
    showReferences,
  );
  const showTypeHierarchyCommand = vscode.commands.registerCommand(
    "solar.showTypeHierarchy",
    showTypeHierarchy,
  );

  context.subscriptions.push(
    formatCommand,
    formatOnSave,
    configListener,
    copySelectorCommand,
    showReferencesCommand,
    showTypeHierarchyCommand,
  );
}

function restartLanguageServer(): Promise<void> {
  clientLifecycle = clientLifecycle
    .then(async () => {
      await stopLanguageServer();

      const config = vscode.workspace.getConfiguration("solarLsp");
      if (config.get<boolean>("enable", true)) {
        await startLanguageServer();
      }
    })
    .catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      console.error("Failed to restart LSP client:", error);
      vscode.window.showErrorMessage(`Failed to restart LSP: ${message}`);
    });
  return clientLifecycle;
}

async function stopLanguageServer(): Promise<void> {
  fallbackFormatter?.dispose();
  fallbackFormatter = undefined;
  activeForgePath = undefined;
  const currentClient = client;
  client = undefined;
  await currentClient?.dispose();
}

async function startLanguageServer() {
  const config = vscode.workspace.getConfiguration("solarLsp");
  const oldSetting = config.inspect<string>("serverPath");
  if ([oldSetting?.globalValue, oldSetting?.workspaceValue, oldSetting?.workspaceFolderValue]
      .some((value) => value !== undefined)) {
    void vscode.window.showWarningMessage(
      "solarLsp.serverPath previously selected standalone Solar and is no longer used. " +
      "Remove it and configure solarLsp.forgePath with a Forge executable; Solar paths are not reinterpreted as Forge paths.",
    );
  }
  const forgePath = await resolveForge(
    launcherForgePath || config.get<string>("forgePath", "forge"),
    vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
  );
  await validateForgeLsp(forgePath);
  const flychecks = config.get("flychecks");
  const codeLens = {
    enable: config.get<boolean>("codeLens.enable", true),
    selectors: config.get<boolean>("codeLens.selectors", true),
    references: config.get<boolean>("codeLens.references", true),
    inheritance: config.get<boolean>("codeLens.inheritance", true),
    clientCommands: true,
  };

  const serverOptions: ServerOptions = {
    command: forgePath,
    args: ["lsp", "--stdio"],
    // The default executable transport is stdio; setting it explicitly appends a second --stdio.
  };

  // Define client options.
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "solidity" }],
    initializationOptions: {
      forgePath,
      flychecks,
      codeLens,
      indexing: {
        exclude: config.get<string[]>("indexing.exclude", []),
        useDefaultExcludes: config.get<boolean>("indexing.useDefaultExcludes", true),
        excludeHiddenDirectories: config.get<boolean>("indexing.excludeHiddenDirectories", true),
        excludeNestedRepositories: config.get<boolean>("indexing.excludeNestedRepositories", true),
      },
    },
    // The pinned solar_lsp registers scoped source/config/remapping watchers dynamically.
  };

  // Create the language client and start it.
  const nextClient = new LanguageClient(
    "solarLsp",
    "Solar LSP",
    serverOptions,
    clientOptions,
  );
  // Keep the compatibility channel visible and record the executable that backs it.
  const outputChannel = nextClient.outputChannel;
  outputChannel.appendLine(`Starting Forge LSP: ${forgePath} lsp --stdio`);
  client = nextClient;
  activeForgePath = forgePath;

  // Start the client. This also launches the server.
  try {
    await nextClient.start();
    outputChannel.appendLine(`Forge LSP started: ${forgePath} lsp --stdio`);
    console.log(`Forge LSP client started: ${forgePath} lsp --stdio`);
    if (!serverSupportsDocumentFormatting()) {
      fallbackFormatter = vscode.languages.registerDocumentFormattingEditProvider(
        { scheme: "file", language: "solidity" },
        { provideDocumentFormattingEdits: async (document) => {
          const edit = await formatDocumentWithForge(document);
          return edit ? [edit] : [];
        } },
      );
    }
  } catch (error) {
    try {
      await nextClient.dispose();
    } catch {
      // Failed starts can also reject shutdown after scheduling process cleanup.
    }
    if (client === nextClient) {
      client = undefined;
      activeForgePath = undefined;
    }
    const message = error instanceof Error ? error.message : String(error);
    console.error("Failed to start LSP client:", error);
    vscode.window.showErrorMessage(`Failed to start LSP: ${message}`);
  }
}

async function copySelector(selector: unknown): Promise<void> {
  if (typeof selector !== "string" || selector.length === 0) {
    return;
  }
  await vscode.env.clipboard.writeText(selector);
}

async function showReferences(argument: unknown): Promise<void> {
  const location = parseCodeLensLocation(argument);
  const runningClient = client;
  if (!location || !runningClient || runningClient.state !== State.Running) {
    return;
  }

  try {
    const result = await runningClient.sendRequest(ReferencesRequest.type, {
      textDocument: { uri: location.uri.toString() },
      position: {
        line: location.position.line,
        character: location.position.character,
      },
      context: { includeDeclaration: false },
    });
    const references = await runningClient.protocol2CodeConverter.asReferences(
      result ?? [],
    );
    await vscode.commands.executeCommand(
      "editor.action.showReferences",
      location.uri,
      location.position,
      references,
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error("Failed to show references:", error);
    vscode.window.showErrorMessage(`Failed to show references: ${message}`);
  }
}

async function showTypeHierarchy(argument: unknown): Promise<void> {
  const location = parseCodeLensLocation(argument);
  if (!location || !argument || typeof argument !== "object") {
    return;
  }

  const direction = (argument as { direction?: unknown }).direction;
  if (direction !== "supertypes" && direction !== "subtypes") {
    return;
  }

  try {
    const range = new vscode.Range(location.position, location.position);
    const editor = await vscode.window.showTextDocument(location.uri, {
      selection: range,
    });
    editor.revealRange(
      range,
      vscode.TextEditorRevealType.InCenterIfOutsideViewport,
    );
    await vscode.commands.executeCommand("editor.showTypeHierarchy");
    await vscode.commands.executeCommand(
      direction === "supertypes"
        ? "editor.showSupertypes"
        : "editor.showSubtypes",
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error("Failed to show type hierarchy:", error);
    vscode.window.showErrorMessage(`Failed to show type hierarchy: ${message}`);
  }
}

function parseCodeLensLocation(
  argument: unknown,
): { uri: vscode.Uri; position: vscode.Position } | undefined {
  if (!argument || typeof argument !== "object") {
    return undefined;
  }

  const candidate = argument as {
    uri?: unknown;
    position?: { line?: unknown; character?: unknown };
  };
  if (
    typeof candidate.uri !== "string" ||
    !candidate.position ||
    typeof candidate.position.line !== "number" ||
    typeof candidate.position.character !== "number" ||
    !Number.isInteger(candidate.position.line) ||
    !Number.isInteger(candidate.position.character) ||
    candidate.position.line < 0 ||
    candidate.position.character < 0
  ) {
    return undefined;
  }

  return {
    uri: vscode.Uri.parse(candidate.uri),
    position: new vscode.Position(
      candidate.position.line,
      candidate.position.character,
    ),
  };
}

async function formatDocument(
  document: vscode.TextDocument,
  expectedVersion = document.version,
): Promise<vscode.TextEdit[]> {
  await clientLifecycle;
  if (
    document.isClosed ||
    document.version !== expectedVersion ||
    !activeForgePath
  ) {
    return [];
  }
  if (!serverSupportsDocumentFormatting()) {
    const edit = await formatDocumentWithForge(document, expectedVersion);
    return edit ? [edit] : [];
  }

  const editorConfig = vscode.workspace.getConfiguration("editor", document);
  const options: vscode.FormattingOptions = {
    tabSize: editorConfig.get<number>("tabSize", 4),
    insertSpaces: editorConfig.get<boolean>("insertSpaces", true),
  };
  const runningClient = client!;
  const edits = await runningClient.sendRequest(DocumentFormattingRequest.type, {
    textDocument: { uri: document.uri.toString() },
    options,
  });
  if (document.isClosed || document.version !== expectedVersion) {
    return [];
  }
  const convertedEdits = await runningClient.protocol2CodeConverter.asTextEdits(edits);
  if (document.isClosed || document.version !== expectedVersion) {
    return [];
  }
  return convertedEdits ?? [];
}

function serverSupportsDocumentFormatting(): boolean {
  return (
    client?.state === State.Running &&
    Boolean(client.initializeResult?.capabilities.documentFormattingProvider)
  );
}

async function formatDocumentWithForge(
  document: vscode.TextDocument,
  expectedVersion = document.version,
): Promise<vscode.TextEdit | undefined> {
  const forgePath = activeForgePath;
  if (!forgePath) {
    return undefined;
  }
  const version = expectedVersion;
  const source = document.getText();
  const root = await formatterRoot(
    document.uri.fsPath,
    vscode.workspace.getWorkspaceFolder(document.uri)?.uri.fsPath,
  );

  return new Promise((resolve) => {
    const forgeProcess = spawn(forgePath, ["fmt", "--raw", "--root", root, "-"], {
      cwd: root,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });

    let stdout = "";
    let stderr = "";

    forgeProcess.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    forgeProcess.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    forgeProcess.on("close", (code) => {
      if (document.version !== version || document.isClosed) {
        resolve(undefined);
        return;
      }
      if (code === 0) {
        const firstLine = document.lineAt(0);
        const lastLine = document.lineAt(document.lineCount - 1);
        const textRange = new vscode.Range(
          firstLine.range.start,
          lastLine.range.end,
        );

        resolve(new vscode.TextEdit(textRange, stdout));
      } else {
        console.error(`forge fmt failed with code ${code}: ${stderr}`);
        vscode.window.showErrorMessage(`Formatting failed: ${stderr}`);
        resolve(undefined);
      }
    });

    forgeProcess.on("error", (error) => {
      console.error("Failed to run forge fmt:", error);
      vscode.window.showErrorMessage(
        `Failed to run forge fmt: ${error.message}`,
      );
      resolve(undefined);
    });

    forgeProcess.stdin.on("error", () => {
      // An early process exit is reported by the error/close handlers above.
    });
    forgeProcess.stdin.end(source);
  });
}

export function deactivate(): Thenable<void> | undefined {
  return clientLifecycle.then(stopLanguageServer);
}
