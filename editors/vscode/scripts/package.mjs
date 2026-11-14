// Stage the VSIX with the bundled client and root license texts, independent of Git symlink support.
import { createVSIX, listFiles } from "@vscode/vsce";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { gunzipSync } from "node:zlib";

const { values } = parseArgs({ options: { "pre-release": { type: "boolean" } } });

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const bundle = join(root, "bundle");
const licenses = ["LICENSE-MIT", "LICENSE-APACHE"];
await mkdir(bundle, { recursive: true });
const staging = await mkdtemp(join(bundle, ".package-"));
try {
  const files = new Set([...await listFiles({ cwd: root }), ...licenses, ".vscodeignore"]);
  for (const file of files) {
    if (file.startsWith("out/") || file.startsWith("node_modules/")) {
      continue;
    }
    const source = licenses.includes(file) ? join(root, "../..", file) : join(root, file);
    const destination = join(staging, file);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(source, destination);
  }

  // Ship the same bundled client that Forge embeds instead of the compiled sources and node_modules.
  await mkdir(join(staging, "out"));
  await writeFile(join(staging, "out/extension.js"), gunzipSync(await readFile(join(root, "dist/extension.js.gz"))));
  await copyFile(join(root, "dist/THIRD_PARTY_NOTICES.txt"), join(staging, "THIRD_PARTY_NOTICES.txt"));

  const manifestPath = join(staging, "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  delete manifest.scripts["vscode:prepublish"];
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
  await createVSIX({ cwd: staging, packagePath: join(bundle, "forge.vsix"), useYarn: false, dependencies: false, preRelease: values["pre-release"] });
} finally {
  await rm(staging, { recursive: true, force: true });
}
