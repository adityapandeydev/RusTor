import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { checksumManifest, findBundles, platformPlan, renderNotices } from "./package-lib.mjs";

test("desktop, frontend and workspace versions agree for packaged releases", async () => {
  const root = new URL("../", import.meta.url);
  const frontend = JSON.parse(await readFile(new URL("apps/desktop/package.json", root), "utf8"));
  const tauri = JSON.parse(await readFile(new URL("apps/desktop/src-tauri/tauri.conf.json", root), "utf8"));
  const cargo = await readFile(new URL("Cargo.toml", root), "utf8");
  const workspace = /\[workspace\.package\]\s+version = "([^"]+)"/.exec(cargo)?.[1];
  assert.equal(frontend.version, tauri.version);
  assert.equal(workspace, tauri.version);
  const bun = await readFile(new URL("bun.lock", root), "utf8");
  assert.equal(/"apps\/desktop":\s*\{\s*"name": "@rustor\/desktop",\s*"version": "([^"]+)"/.exec(bun)?.[1], tauri.version);
  const locked = await readFile(new URL("Cargo.lock", root), "utf8");
  for (const name of ["rustor-desktop", "rustor-search", "rustor-torrent"])
    assert.equal(new RegExp(`name = "${name}"\\nversion = "([^"]+)"`).exec(locked)?.[1], tauri.version);
});

test("supported native targets select installer formats and reject unknown architectures", () => {
  assert.deepEqual(platformPlan("x86_64-pc-windows-msvc").bundles, ["nsis"]);
  assert.deepEqual(platformPlan("x86_64-unknown-linux-gnu").bundles, ["appimage", "deb"]);
  assert.equal(platformPlan("aarch64-apple-darwin").arch, "arm64");
  assert.throws(() => platformPlan("armv7-unknown-linux-gnueabihf"));
});

test("notices preserve actual license text and reject omitted fonts or unreviewed licenses", () => {
  const rust = { licenses: [{ id: "MIT", text: "Rust copyright and license", used_by: [{ crate: { name: "fixture", version: "1.0" } }] }] };
  const frontend = ["react", "react-dom", "scheduler", "lucide-react", "@tauri-apps/api", "@fontsource-variable/manrope"]
    .map((name) => ({ name, version: "1.0", identifier: "MIT", text: `${name} copyright and license` }));
  const notice = renderNotices(rust, frontend);
  assert.ok(notice.includes("Rust copyright and license"));
  assert.ok(notice.includes("@fontsource-variable/manrope copyright and license"));
  assert.throws(() => renderNotices(rust, frontend.slice(0, -1)));
  assert.throws(() => renderNotices(rust, [{ ...frontend[0], identifier: "UNKNOWN" }, ...frontend.slice(1)]));
  assert.throws(() => renderNotices({ licenses: [] }, frontend));
  const revision = "f119a999657d66fe2c2322613428241ce8a69f23";
  rust.licenses[0].used_by[0].crate.source = `git+https://github.com/ikatson/librqbit-utp?rev=${revision}#${revision}`;
  assert.ok(renderNotices(rust, frontend).includes(`https://github.com/ikatson/librqbit-utp/archive/${revision}.tar.gz`));
});

test("artifact collection rejects stale versions, absent formats, and ambiguous installers", async (t) => {
  const dir = await mkdtemp(join(tmpdir(), "rustor-package-test-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, "nsis"));
  await writeFile(join(dir, "nsis/RusTor_0.0.9_x64-setup.exe"), "old");
  await assert.rejects(findBundles(dir, ["nsis"], "0.1.1", 0));
  const installer = join(dir, "nsis/RusTor_0.1.1_x64-setup.exe");
  await writeFile(installer, "new");
  assert.deepEqual(await findBundles(dir, ["nsis"], "0.1.1", 0), [installer]);
  await assert.rejects(findBundles(dir, ["nsis"], "0.1.1", Date.now() + 10000));
  await writeFile(join(dir, "nsis/Other_0.1.1_x64-setup.exe"), "duplicate");
  await assert.rejects(findBundles(dir, ["nsis"], "0.1.1", 0));
});

test("checksums cover distributed files and change when an artifact changes", async (t) => {
  const dir = await mkdtemp(join(tmpdir(), "rustor-checksum-test-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await writeFile(join(dir, "installer.exe"), "abc");
  await writeFile(join(dir, "SHA256SUMS"), "old manifest");
  const first = await checksumManifest(dir);
  assert.equal(first, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  installer.exe\n");
  await writeFile(join(dir, "installer.exe"), "changed");
  assert.notEqual(await checksumManifest(dir), first);
  assert.equal(await readFile(join(dir, "SHA256SUMS"), "utf8"), "old manifest");
});
