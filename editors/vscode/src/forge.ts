import { execFile } from "node:child_process";
import { constants } from "node:fs";
import { access, stat } from "node:fs/promises";
import * as path from "node:path";

const installationHelp =
  "Install or upgrade Foundry to a build with LSP support (https://getfoundry.sh), " +
  "then verify `forge lsp --stdio --help`. For a source build, use `cargo build --locked -p forge --bin forge`. " +
  "Set solarLsp.forgePath to that Forge executable if it is not on PATH.";

/** Resolve once so the server, formatter and Forge checks use the same executable. */
export async function resolveForge(
  configuredPath: string,
  cwd = process.cwd(),
  env = process.env,
): Promise<string> {
  const command = configuredPath.trim() || "forge";
  const hasDirectory = path.isAbsolute(command) || /[/\\]/.test(command);
  const candidates = hasDirectory
    ? [path.resolve(cwd, command)]
    : (env.PATH ?? env.Path ?? "")
        .split(path.delimiter)
        .filter(Boolean)
        .map((directory) => path.resolve(cwd, directory, command));
  const extensions = process.platform === "win32" && !path.extname(command)
    ? [".exe", ".com"]
    : [""];
  for (const candidate of candidates) {
    for (const extension of extensions) {
      const executable = candidate + extension;
      try {
        await access(executable, constants.X_OK);
        if ((await stat(executable)).isFile()) {
          return executable;
        }
      } catch {
        // Continue searching PATH for an executable file.
      }
    }
  }
  throw new Error(
    `Forge executable ${JSON.stringify(command)} was not found or is not executable. ${installationHelp}`,
  );
}

/** A successful --version does not establish that this build includes LSP. */
export async function validateForgeLsp(forgePath: string): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    execFile(
      forgePath,
      ["lsp", "--stdio", "--help"],
      { windowsHide: true, timeout: 10_000, maxBuffer: 1024 * 1024 },
      (error) => {
        if (error) {
          reject(new Error(
            `Forge at ${JSON.stringify(forgePath)} does not support \`forge lsp --stdio\` ` +
            `or could not run its LSP capability check. ${installationHelp} (${error.message})`,
          ));
        } else {
          resolve();
        }
      },
    );
  });
}

/** Locate the document's own Foundry project, including nested projects. */
export async function formatterRoot(
  documentPath: string,
  workspaceRoot?: string,
): Promise<string> {
  let directory = path.dirname(documentPath);
  for (;;) {
    try {
      if ((await stat(path.join(directory, "foundry.toml"))).isFile()) {
        return directory;
      }
    } catch {
      // A workspace need not contain a Foundry configuration.
    }
    const parent = path.dirname(directory);
    if (parent === directory) {
      return workspaceRoot ?? path.dirname(documentPath);
    }
    directory = parent;
  }
}

/** The legacy save hook yields to VS Code's formatter to avoid applying edits twice. */
export function shouldFormatOnSave(
  enabled: boolean,
  legacyFormatOnSave: boolean,
  editorFormatOnSave: boolean,
): boolean {
  return enabled && legacyFormatOnSave && !editorFormatOnSave;
}
