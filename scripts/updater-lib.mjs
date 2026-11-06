import { createHash, createPublicKey, verify } from "node:crypto";
import { copyFile, lstat, readFile, readdir, stat, writeFile } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { join } from "node:path";

const plans = {
  "x86_64-pc-windows-msvc": { platform: "windows-x86_64", folder: "nsis", extension: ".exe", arch: "x64", bundles: ["nsis"] },
  "x86_64-unknown-linux-gnu": { platform: "linux-x86_64", folder: "appimage", extension: ".AppImage", arch: "x64", bundles: ["appimage", "deb"] },
  "aarch64-apple-darwin": { platform: "darwin-aarch64", folder: "macos", extension: ".app.tar.gz", arch: "arm64", bundles: ["dmg", "app"] },
  "x86_64-apple-darwin": { platform: "darwin-x86_64", folder: "macos", extension: ".app.tar.gz", arch: "x64", bundles: ["dmg", "app"] },
};

export function updaterPlan(target) {
  if (!Object.hasOwn(plans, target)) throw new Error(`Unsupported updater target: ${target}`);
  return plans[target];
}

function base64(value) {
  if (typeof value !== "string" || value.length > 8192 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value) || !value)
    throw new Error("Invalid updater base64 encoding");
  return Buffer.from(value, "base64");
}

function keyAndSignature(signature, pubkey) {
  // Mirrors minisign's documented envelope and trusted-comment verification;
  // Node/OpenSSL supplies Ed25519 and BLAKE2b, not a custom crypto primitive.
  // https://github.com/jedisct1/rust-minisign-verify/blob/master/src/lib.rs
  const keyLines = base64(pubkey.trim()).toString("utf8").trimEnd().split(/\r?\n/);
  const lines = base64(signature.trim()).toString("utf8").trimEnd().split(/\r?\n/);
  if (keyLines.length !== 2 || !keyLines[0].startsWith("untrusted comment: ") || lines.length !== 4 || !lines[0].startsWith("untrusted comment: ") || !lines[2].startsWith("trusted comment: "))
    throw new Error("Invalid updater minisign text");
  const key = base64(keyLines[1]);
  const signed = base64(lines[1]);
  const global = base64(lines[3]);
  // Tauri's current signer emits prehashed minisign (ED). Deliberately reject
  // legacy Ed signatures instead of buffering entire installers in memory.
  if (key.length !== 42 || !["Ed", "ED"].includes(key.subarray(0, 2).toString()) || signed.length !== 74 || signed.subarray(0, 2).toString() !== "ED" || global.length !== 64)
    throw new Error("Unsupported updater signature format (prehashed minisign required)");
  if (!key.subarray(2, 10).equals(signed.subarray(2, 10))) throw new Error("Updater signing key mismatch");
  const publicKey = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), key.subarray(10)]), type: "spki", format: "der" });
  const raw = signed.subarray(10);
  if (!verify(null, Buffer.concat([raw, Buffer.from(lines[2].slice(17))]), publicKey, global))
    throw new Error("Invalid updater trusted-comment signature");
  const versions = lines[2].slice(17).split("\t").filter((field) => field.startsWith("version:")).map((field) => field.slice(8));
  return { publicKey, raw, versions };
}

function verifyDigest(digest, { publicKey, raw }) {
  if (!verify(null, digest, publicKey, raw)) throw new Error("Invalid updater artifact signature");
}

export function verifyUpdater(bytes, signature, pubkey) {
  verifyDigest(createHash("blake2b512").update(bytes).digest(), keyAndSignature(signature, pubkey));
}

async function verifyFile(path, signature, pubkey, version) {
  const parsed = keyAndSignature(signature, pubkey);
  if (parsed.versions.length !== 1 || parsed.versions[0] !== version)
    throw new Error("Updater signature must bind the exact release version");
  if (!(await lstat(path)).isFile()) throw new Error("Updater artifact must be a regular file");
  const hash = createHash("blake2b512");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  verifyDigest(hash.digest(), parsed);
}

function versionNumber(version) {
  if (typeof version !== "string" || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version))
    throw new Error("Updater expects a stable MAJOR.MINOR.PATCH version");
}

function artifactName(file, plan, version) {
  if (typeof file !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(file) || !file.includes(`_${version}_`) || !file.endsWith(plan.extension))
    throw new Error("Invalid updater artifact name or format");
}

export async function collectUpdater(directory, output, target, version, since, pubkey) {
  versionNumber(version);
  const plan = updaterPlan(target);
  const folder = join(directory, plan.folder);
  const candidates = [];
  for (const entry of await readdir(folder, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(plan.extension)) continue;
    if (plan.folder !== "macos" && !entry.name.includes(`_${version}_`)) continue;
    const path = join(folder, entry.name);
    if ((await stat(path)).mtimeMs >= since) candidates.push(path);
  }
  if (candidates.length !== 1) throw new Error(`Expected one fresh updater artifact, found ${candidates.length}`);
  const source = candidates[0];
  const signaturePath = `${source}.sig`;
  if ((await stat(signaturePath)).mtimeMs < since) throw new Error("Stale updater signature");
  const signature = (await readFile(signaturePath, "utf8")).trim();
  await verifyFile(source, signature, pubkey, version);
  // macOS's default app archive name collides across the two native runners.
  const file = plan.folder === "macos" ? `RusTor_${version}_${plan.arch}.app.tar.gz` : source.split(/[\\/]/).at(-1);
  artifactName(file, plan, version);
  await copyFile(source, join(output, file));
  await writeFile(join(output, `${file}.sig`), `${signature}\n`);
  const info = { target, version, file };
  await writeFile(join(output, "UPDATER-INFO.json"), `${JSON.stringify(info, null, 2)}\n`);
  return info;
}

export async function buildUpdateManifest({ dirs, pubkey, version, notes = "", pubDate }) {
  versionNumber(version);
  if (typeof notes !== "string" || notes.length > 100000 || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(pubDate) || !Number.isFinite(Date.parse(pubDate)))
    throw new Error("Invalid updater release notes or UTC publication date");
  if (!Array.isArray(dirs) || dirs.length !== Object.keys(plans).length) throw new Error("All four updater targets are required exactly once");
  const platforms = {};
  let commit;
  const files = new Set();
  for (const dir of dirs) {
    const info = JSON.parse(await readFile(join(dir, "UPDATER-INFO.json"), "utf8"));
    const build = JSON.parse(await readFile(join(dir, "BUILD-INFO.json"), "utf8"));
    const plan = updaterPlan(info.target);
    if (platforms[plan.platform] || info.version !== version || build.version !== version || build.target !== info.target || build.dirty !== false || !/^[a-f0-9]{40}$/.test(build.commit) || (commit && build.commit !== commit))
      throw new Error("Updater inputs contain duplicate, mismatched, dirty or mixed-commit builds");
    commit = build.commit;
    artifactName(info.file, plan, version);
    if (files.has(info.file)) throw new Error("Updater asset filenames must be unique across platforms");
    files.add(info.file);
    const signature = (await readFile(join(dir, `${info.file}.sig`), "utf8")).trim();
    await verifyFile(join(dir, info.file), signature, pubkey, version);
    platforms[plan.platform] = {
      signature,
      url: `https://github.com/adityapandeydev/RusTor/releases/download/v${version}/${encodeURIComponent(info.file)}`,
    };
  }
  return { version, notes, pub_date: pubDate, platforms };
}
