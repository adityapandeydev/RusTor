import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { buildUpdateManifest, collectUpdater, updaterPlan, verifyUpdater } from "./updater-lib.mjs";

// Ephemeral test keys, never a production updater identity. Tauri wraps minisign
// text in base64; minisign signs a BLAKE2b digest and the trusted comment.
function signer() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const id = randomBytes(8);
  const raw = publicKey.export({ type: "spki", format: "der" }).subarray(-32);
  const pubkey = Buffer.from(`untrusted comment: test\n${Buffer.concat([Buffer.from("Ed"), id, raw]).toString("base64")}\n`).toString("base64");
  return {
    pubkey,
    signature(bytes, comment = "timestamp:0\tversion:0.2.0") {
      const signature = sign(null, createHash("blake2b512").update(bytes).digest(), privateKey);
      const global = sign(null, Buffer.concat([signature, Buffer.from(comment)]), privateKey);
      return Buffer.from(`untrusted comment: test\n${Buffer.concat([Buffer.from("ED"), id, signature]).toString("base64")}\ntrusted comment: ${comment}\n${global.toString("base64")}\n`).toString("base64");
    },
  };
}

test("updater plans distinguish installed formats and architecture-specific macOS archives", () => {
  assert.equal(updaterPlan("x86_64-pc-windows-msvc").platform, "windows-x86_64");
  assert.equal(updaterPlan("x86_64-unknown-linux-gnu").extension, ".AppImage");
  assert.deepEqual(updaterPlan("aarch64-apple-darwin").bundles, ["dmg", "app"]);
  assert.throws(() => updaterPlan("armv7-linux"));
});

test("signature verification rejects changed bytes, another key, malformed encoding and forged comments", () => {
  const key = signer();
  const bytes = Buffer.from("installer bytes");
  const signature = key.signature(bytes);
  assert.doesNotThrow(() => verifyUpdater(bytes, signature, key.pubkey));
  assert.throws(() => verifyUpdater(Buffer.from("tampered"), signature, key.pubkey));
  assert.throws(() => verifyUpdater(bytes, signature, signer().pubkey));
  assert.throws(() => verifyUpdater(bytes, "https://example.com/file.sig", key.pubkey));
  assert.throws(() => verifyUpdater(bytes, signature, "not-a-key"));
  const forged = Buffer.from(Buffer.from(signature, "base64").toString().replace("timestamp:0", "timestamp:1")).toString("base64");
  assert.throws(() => verifyUpdater(bytes, forged, key.pubkey));
});

test("artifact collection rejects stale, missing and ambiguous files and renames macOS archives", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "rustor-updater-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const key = signer();
  const bundle = join(root, "bundle/macos");
  const output = join(root, "output");
  await mkdir(bundle, { recursive: true });
  await mkdir(output);
  const archive = join(bundle, "RusTor.app.tar.gz");
  await writeFile(archive, "app archive");
  await assert.rejects(collectUpdater(join(root, "bundle"), output, "aarch64-apple-darwin", "0.2.0", 0, key.pubkey));
  await writeFile(`${archive}.sig`, key.signature(Buffer.from("app archive")));
  await assert.rejects(collectUpdater(join(root, "bundle"), output, "aarch64-apple-darwin", "0.2.0", Date.now() + 10000, key.pubkey));
  const info = await collectUpdater(join(root, "bundle"), output, "aarch64-apple-darwin", "0.2.0", 0, key.pubkey);
  assert.equal(info.file, "RusTor_0.2.0_arm64.app.tar.gz");
  assert.equal(await readFile(join(output, info.file), "utf8"), "app archive");
  await writeFile(join(bundle, "Other.app.tar.gz"), "another");
  await assert.rejects(collectUpdater(join(root, "bundle"), output, "aarch64-apple-darwin", "0.2.0", 0, key.pubkey));
});

async function releaseFixture(t, version = "0.2.0") {
  const root = await mkdtemp(join(tmpdir(), "rustor-feed-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const key = signer();
  const targets = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-apple-darwin"];
  const dirs = [];
  for (const target of targets) {
    const dir = join(root, target);
    await mkdir(dir);
    const plan = updaterPlan(target);
    const file = `RusTor_${version}_${plan.arch}${plan.extension}`;
    const bytes = Buffer.from(target);
    await writeFile(join(dir, file), bytes);
    await writeFile(join(dir, `${file}.sig`), key.signature(bytes, `timestamp:0\tversion:${version}`));
    await writeFile(join(dir, "UPDATER-INFO.json"), JSON.stringify({ target, version, file }));
    await writeFile(join(dir, "BUILD-INFO.json"), JSON.stringify({ target, version, commit: "a".repeat(40), dirty: false }));
    dirs.push(dir);
  }
  return { dirs, pubkey: key.pubkey, version, notes: "Test release", pubDate: "2026-11-06T12:00:00Z" };
}

test("complete release feed uses verified bytes and immutable GitHub asset URLs", async (t) => {
  const fixture = await releaseFixture(t);
  const manifest = await buildUpdateManifest(fixture);
  assert.equal(manifest.version, "0.2.0");
  assert.equal(Object.keys(manifest.platforms).length, 4);
  assert.equal(manifest.platforms["windows-x86_64"].url, "https://github.com/adityapandeydev/RusTor/releases/download/v0.2.0/RusTor_0.2.0_x64.exe");
  assert.ok(manifest.platforms["darwin-aarch64"].signature);
  assert.equal(manifest.pub_date, fixture.pubDate);
});

test("release feed refuses incomplete sets, duplicates, wrong version, dirty/mixed builds and corrupt bytes", async (t) => {
  const fixture = await releaseFixture(t);
  await assert.rejects(buildUpdateManifest({ ...fixture, dirs: fixture.dirs.slice(1) }));
  await assert.rejects(buildUpdateManifest({ ...fixture, dirs: [...fixture.dirs, fixture.dirs[0]] }));
  await assert.rejects(buildUpdateManifest({ ...fixture, version: "0.2.1" }));
  await assert.rejects(buildUpdateManifest({ ...fixture, version: "../evil" }));
  await assert.rejects(buildUpdateManifest({ ...fixture, pubDate: "invalid" }));
  const infoPath = join(fixture.dirs[0], "BUILD-INFO.json");
  const original = JSON.parse(await readFile(infoPath, "utf8"));
  await writeFile(infoPath, JSON.stringify({ ...original, dirty: true }));
  await assert.rejects(buildUpdateManifest(fixture));
  await writeFile(infoPath, JSON.stringify({ ...original, commit: "b".repeat(40) }));
  await assert.rejects(buildUpdateManifest(fixture));
  await writeFile(infoPath, JSON.stringify(original));
  const info = JSON.parse(await readFile(join(fixture.dirs[0], "UPDATER-INFO.json"), "utf8"));
  await writeFile(join(fixture.dirs[0], info.file), "corrupt");
  await assert.rejects(buildUpdateManifest(fixture));
});

test("feed input rejects traversal paths and wrong package formats", async (t) => {
  const fixture = await releaseFixture(t);
  const path = join(fixture.dirs[0], "UPDATER-INFO.json");
  const original = JSON.parse(await readFile(path, "utf8"));
  for (const file of ["../installer.exe", "C:\\installer.exe", "installer.zip", "installer\n.exe"]) {
    await writeFile(path, JSON.stringify({ ...original, file }));
    await assert.rejects(buildUpdateManifest(fixture));
  }
});

test("collection accepts a real Tauri CLI signature and rejects a differently signed release version", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "rustor-tauri-signer-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const cli = fileURLToPath(new URL("../apps/desktop/node_modules/@tauri-apps/cli/tauri.js", import.meta.url));
  function run(args) {
    // Capture signer output: it can contain key material and must never reach CI logs.
    const env = { ...process.env };
    for (const name of ["TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PATH", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"]) delete env[name];
    const result = spawnSync(process.execPath, [cli, "signer", ...args], { encoding: "utf8", timeout: 30000, env });
    assert.equal(result.status, 0, "Tauri signing fixture command failed (output withheld)");
  }
  const key = join(root, "test.key");
  run(["generate", "--ci", "-w", key, "-p", "fixture-only"]);
  const bundle = join(root, "bundle/nsis");
  const output = join(root, "output");
  await mkdir(bundle, { recursive: true });
  await mkdir(output);
  const path = join(bundle, "RusTor_0.2.0_x64-setup.exe");
  await writeFile(path, "real CLI signed fixture");
  const pubkey = (await readFile(`${key}.pub`, "utf8")).trim();
  run(["sign", "-f", key, "-p", "fixture-only", "--app-version", "0.2.0", path]);
  await collectUpdater(join(root, "bundle"), output, "x86_64-pc-windows-msvc", "0.2.0", 0, pubkey);
  run(["sign", "-f", key, "-p", "fixture-only", "--app-version", "0.1.9", path]);
  await assert.rejects(collectUpdater(join(root, "bundle"), output, "x86_64-pc-windows-msvc", "0.2.0", 0, pubkey), /exact release version/);
});

test("manifest CLI writes only a fully verified feed and preserves an existing output", async (t) => {
  const config = JSON.parse(await readFile(new URL("../apps/desktop/src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const fixture = await releaseFixture(t, config.version);
  const root = join(fixture.dirs[0], "..");
  // Keep notes/output outside the packages root, which contains only native dirs.
  const scratch = await mkdtemp(join(tmpdir(), "rustor-feed-cli-"));
  t.after(() => rm(scratch, { recursive: true, force: true }));
  const notes = join(scratch, "notes.txt");
  const output = join(scratch, "latest.json");
  await writeFile(notes, "CLI fixture");
  const cli = fileURLToPath(new URL("./updater-manifest.mjs", import.meta.url));
  const args = [cli, "--packages", root, "--notes", notes, "--output", output];
  const env = { ...process.env, RUSTOR_UPDATER_PUBLIC_KEY: fixture.pubkey };
  const first = spawnSync(process.execPath, args, { env, encoding: "utf8" });
  assert.equal(first.status, 0, first.stderr);
  const original = await readFile(output, "utf8");
  assert.equal(JSON.parse(original).notes, "CLI fixture");
  assert.notEqual(spawnSync(process.execPath, args, { env, encoding: "utf8" }).status, 0);
  assert.equal(await readFile(output, "utf8"), original);
  const info = JSON.parse(await readFile(join(fixture.dirs[0], "UPDATER-INFO.json"), "utf8"));
  await writeFile(join(fixture.dirs[0], info.file), "corrupt");
  const rejectedOutput = join(scratch, "rejected.json");
  const rejected = spawnSync(process.execPath, [...args.slice(0, -1), rejectedOutput], { env, encoding: "utf8" });
  assert.notEqual(rejected.status, 0);
  await assert.rejects(readFile(rejectedOutput));
});

test("signed packaging fails before building when production credentials are absent", () => {
  const env = { ...process.env };
  delete env.TAURI_SIGNING_PRIVATE_KEY;
  delete env.RUSTOR_UPDATER_PUBLIC_KEY;
  const cli = fileURLToPath(new URL("./package.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [cli, "--updater"], { env, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Signed updater builds require/);
});

test("signed packaging refuses a CI public key that differs from the app trust key", () => {
  const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: "not-a-production-key", RUSTOR_UPDATER_PUBLIC_KEY: "mismatched-public-key" };
  const cli = fileURLToPath(new URL("./package.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [cli, "--updater"], { env, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /must match the trusted key embedded in the app/);
});
