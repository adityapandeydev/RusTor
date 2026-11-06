import { spawnSync } from "node:child_process";
import { appendFile, copyFile, mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checksumManifest, findBundles, platformPlan, renderNotices } from "./package-lib.mjs";
import { collectUpdater, updaterPlan } from "./updater-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function run(args, cwd = root, capture = false) {
  const result = spawnSync(args[0], args.slice(1), { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${args[0]} exited with status ${result.status}`);
  return result.stdout;
}

const args = process.argv.slice(2);
const updater = args.includes("--updater");
if (updater) args.splice(args.indexOf("--updater"), 1);
if (updater && (!process.env.TAURI_SIGNING_PRIVATE_KEY || !process.env.RUSTOR_UPDATER_PUBLIC_KEY))
  throw new Error("Signed updater builds require TAURI_SIGNING_PRIVATE_KEY and RUSTOR_UPDATER_PUBLIC_KEY");
const host = run(["rustc", "-vV"], root, true).match(/^host: (.+)$/m)?.[1].trim();
const target = args.length === 2 && args[0] === "--target" ? args[1] : host;
if (args.length !== 0 && !(args.length === 2 && args[0] === "--target"))
  throw new Error("Usage: bun run desktop:package [--target NATIVE_TARGET] [--updater]");
if (target !== host || process.env.CARGO_BUILD_TARGET) throw new Error("Package on a native runner without CARGO_BUILD_TARGET; cross-builds are not configured");
const plan = platformPlan(target);
const bundles = updater ? updaterPlan(target).bundles : plan.bundles;
const config = JSON.parse(await readFile(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
const version = config.version;
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Packaging expects a numeric MAJOR.MINOR.PATCH version");
const metadata = JSON.parse(run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], root, true));
const targetDir = metadata.target_directory;
const packageRoot = join(targetDir, "packages", target);
await mkdir(packageRoot, { recursive: true });
const output = await mkdtemp(join(packageRoot, `${version}-`));
const resources = await mkdtemp(join(packageRoot, "resources-"));

run(["cargo", "audit", "--file", join(root, "Cargo.lock")]);
run(["bun", "audit"]);
run(["bun", "run", "build"]);
const rustReport = join(resources, "rust-licenses.json");
run(["cargo", "about", "generate", "--workspace", "--all-features", "--locked", "--fail", "--format", "json", "--output-file", rustReport]);
const rust = JSON.parse(await readFile(rustReport, "utf8"));
const frontend = JSON.parse(await readFile(join(root, "apps/desktop/dist/licenses.json"), "utf8"));
// Keep the bundler source on the project's volume: Windows resource globbing
// cannot reliably resolve an absolute path on a different target-directory drive.
const notice = join(root, "apps/desktop/dist/THIRD_PARTY_NOTICES.txt");
const extraNotices = [];
const seen = new Set();
for (const license of rust.licenses) for (const { crate } of license.used_by) {
  const key = `${crate.name} ${crate.version}`;
  if (seen.has(key)) continue;
  seen.add(key);
  if (!crate.manifest_path) throw new Error(`Missing source location for ${key}`);
  const source = dirname(crate.manifest_path);
  for (const entry of await readdir(source, { withFileTypes: true }))
    if (entry.isFile() && /^NOTICE(?:\.(?:txt|md))?$/i.test(entry.name))
      extraNotices.push(`${key} — ${entry.name}\n${await readFile(join(source, entry.name), "utf8")}`);
}
await writeFile(notice, renderNotices(rust, frontend) + extraNotices.join("\n\n"));
const override = join(resources, "bundle-config.json");
await writeFile(override, JSON.stringify({
  build: { beforeBuildCommand: "" },
  bundle: { active: true, createUpdaterArtifacts: updater, resources: { "../dist/THIRD_PARTY_NOTICES.txt": "THIRD_PARTY_NOTICES.txt" } },
}));
const started = Date.now() - 1000;
// Updater signatures authenticate update bytes; they are NOT OS code signing.
// The default preview route must not demand production signing credentials.
run(["bun", "run", "tauri", "build", "--ci", ...(updater ? [] : ["--no-sign"]), "--config", override, "--bundles", bundles.join(","), "--", "--locked"], join(root, "apps/desktop"));
for (const bundle of await findBundles(join(targetDir, "release/bundle"), plan.bundles, version, started))
  await copyFile(bundle, join(output, basename(bundle)));
if (updater)
  await collectUpdater(join(targetDir, "release/bundle"), output, target, version, started, process.env.RUSTOR_UPDATER_PUBLIC_KEY);
await copyFile(notice, join(output, "THIRD_PARTY_NOTICES.txt"));
for (const name of ["LICENSE", "README.md"]) await copyFile(join(root, name), join(output, name));
if (target.includes("windows")) {
  const standalone = join(resources, "standalone");
  await mkdir(standalone);
  await copyFile(join(targetDir, "release/rustor-desktop.exe"), join(standalone, "RusTor.exe"));
  for (const name of ["LICENSE", "README.md", "THIRD_PARTY_NOTICES.txt"])
    await copyFile(join(output, name), join(standalone, name));
  run(["tar", "-a", "-c", "-f", join(output, `RusTor_${version}_${plan.arch}-standalone.zip`), "-C", standalone, "."]);
}
await writeFile(join(output, "BUILD-INFO.json"), JSON.stringify({
  application: "RusTor", version, target, commit: run(["git", "rev-parse", "HEAD"], root, true).trim(),
  dirty: run(["git", "status", "--porcelain"], root, true).trim().length > 0,
  signing: "Unsigned preview; macOS notarization and public-release signing are not configured.",
  updater_signing: updater ? "Verified against RUSTOR_UPDATER_PUBLIC_KEY; not OS code signing." : "Not enabled.",
}, null, 2) + "\n");
await writeFile(join(output, "SHA256SUMS"), await checksumManifest(output));
if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT, `packages-dir=${output}\n`);
console.log(`Packages and checksums: ${output}`);
