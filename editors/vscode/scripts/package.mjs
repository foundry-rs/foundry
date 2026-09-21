// Stage the VSIX with root license texts, independent of Git symlink support.
import { createVSIX, listFiles } from "@vscode/vsce";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
if (args.length > 0) {
  throw new Error(`package.mjs does not accept arguments: ${args.join(" ")}`);
}

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const bundle = join(root, "bundle");
const licenses = ["LICENSE-MIT", "LICENSE-APACHE"];
await mkdir(bundle, { recursive: true });
const staging = await mkdtemp(join(bundle, ".package-"));
try {
  const files = new Set([...await listFiles({ cwd: root }), ...licenses, ".vscodeignore"]);
  for (const file of files) {
    const source = licenses.includes(file) ? join(root, "../..", file) : join(root, file);
    const destination = join(staging, file);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(source, destination);
  }

  // npm run package has already compiled the client before staging its runtime files.
  const manifestPath = join(staging, "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  delete manifest.scripts["vscode:prepublish"];
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
  await createVSIX({ cwd: staging, packagePath: join(bundle, "solar-lsp.vsix"), useYarn: false });
} finally {
  await rm(staging, { recursive: true, force: true });
}
