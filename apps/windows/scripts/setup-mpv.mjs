// Pinned, baseline-CPU LGPL builds. Run once on Windows before dev/build.
import { createHash } from "node:crypto";
import { readFile, writeFile, mkdir, readdir, copyFile, mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import SevenZip from "7z-wasm";

const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(await readFile(new URL("./mpv-runtime.json", import.meta.url), "utf8"));
const arch = process.argv[2] ?? ({ x64: "x86_64", arm64: "aarch64" })[process.arch];
if (!manifest[arch]) throw new Error("Use x86_64 or aarch64 (matching your Rust build target).");
const dest = path.join(root, "src-tauri", "lib");
const receipt = JSON.stringify({ recipe: 2, arch, assets: manifest[arch] }, null, 2);
try {
  const saved = JSON.parse(await readFile(path.join(dest, "runtime.json"), "utf8"));
  if (saved.receipt === receipt && Object.keys(saved.files).length >= 2) {
    let valid = true;
    for (const [name, hash] of Object.entries(saved.files)) {
      valid &&= createHash("sha256").update(await readFile(path.join(dest, name))).digest("hex") === hash;
    }
    if (valid) { console.log(`libmpv (${arch}) is ready.`); process.exit(0); }
  }
} catch { /* first setup or missing files */ }

const temp = await mkdtemp(path.join(os.tmpdir(), "fluxtv-mpv-"));
try {
  const extracted = path.join(temp, "extracted");
  await mkdir(extracted);
  for (const [component, asset] of Object.entries(manifest[arch])) {
    console.log(`Downloading ${component} (${arch})…`);
    const response = await fetch(asset.url);
    if (!response.ok) throw new Error(`Download failed: HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (createHash("sha256").update(bytes).digest("hex") !== asset.sha256) {
      throw new Error(`Checksum mismatch for ${component}`);
    }
    const name = path.basename(new URL(asset.url).pathname);
    await writeFile(path.join(temp, name), bytes);
    const output = path.join(extracted, component);
    await mkdir(output);
    const seven = await SevenZip({ print: () => {}, printErr: () => {} });
    seven.FS.mkdir("/input");
    seven.FS.mkdir("/output");
    seven.FS.mount(seven.NODEFS, { root: temp }, "/input");
    seven.FS.mount(seven.NODEFS, { root: output }, "/output");
    const status = seven.callMain(["x", `/input/${name}`, "-o/output", "-y"]);
    if (status !== 0) throw new Error(`Archive extraction failed for ${component}: ${status}`);
    seven.FS.unmount("/input");
    seven.FS.unmount("/output");
  }
  await mkdir(dest, { recursive: true });
  const files = {};
  async function collect(dir, prefix = "") {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const source = path.join(dir, entry.name);
      if (entry.isDirectory()) await collect(source, `${prefix}${entry.name}-`);
      else if (entry.name.endsWith(".dll") || /license|copying|copyright/i.test(entry.name)) {
        const name = entry.name.endsWith(".dll") ? entry.name : `notices/${prefix}${entry.name}`;
        await mkdir(path.dirname(path.join(dest, name)), { recursive: true });
        await copyFile(source, path.join(dest, name));
        files[name] = createHash("sha256").update(await readFile(source)).digest("hex");
      }
    }
  }
  await collect(extracted);
  if (!files["libmpv-2.dll"] || !files["libmpv-wrapper.dll"]) throw new Error("Runtime archives are missing DLLs.");
  await copyFile(new URL("./THIRD-PARTY.md", import.meta.url), path.join(dest, "THIRD-PARTY.md"));
  await copyFile(new URL("./LICENSE.LGPL", import.meta.url), path.join(dest, "notices", "mpv-ffmpeg-LICENSE.LGPL"));
  await copyFile(new URL("./COPYING.FFMPEG.LGPL", import.meta.url), path.join(dest, "notices", "ffmpeg-LICENSE.LGPL"));
  await copyFile(new URL("./COPYRIGHT.mpv", import.meta.url), path.join(dest, "notices", "mpv-Copyright"));
  await writeFile(path.join(dest, "runtime.json"), JSON.stringify({ receipt, files }, null, 2));
  console.log(`libmpv (${arch}) is ready in src-tauri/lib. Licenses and source references are included.`);
} finally {
  await rm(temp, { recursive: true, force: true });
}
