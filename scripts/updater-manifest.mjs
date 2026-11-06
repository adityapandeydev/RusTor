import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { buildUpdateManifest } from "./updater-lib.mjs";

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === "--help") {
  console.log("Usage: bun run updater:manifest --packages DIRECTORY --notes FILE --output FILE\nRequires RUSTOR_UPDATER_PUBLIC_KEY. Verifies all four native package directories before writing latest.json. Does not upload or publish anything.");
} else {
  if (args.length !== 6 || args[0] !== "--packages" || args[2] !== "--notes" || args[4] !== "--output")
    throw new Error("Expected --packages DIRECTORY --notes FILE --output FILE (see --help)");
  if (!process.env.RUSTOR_UPDATER_PUBLIC_KEY) throw new Error("RUSTOR_UPDATER_PUBLIC_KEY is required");
  const packages = resolve(args[1]);
  const entries = await readdir(packages, { withFileTypes: true });
  if (entries.some((entry) => !entry.isDirectory())) throw new Error("Packages root must contain only the four native package directories");
  const { version } = JSON.parse(await readFile(new URL("../apps/desktop/src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const manifest = await buildUpdateManifest({
    dirs: entries.map((entry) => join(packages, entry.name)),
    pubkey: process.env.RUSTOR_UPDATER_PUBLIC_KEY,
    version,
    notes: await readFile(resolve(args[3]), "utf8"),
    pubDate: new Date().toISOString(),
  });
  const output = resolve(args[5]);
  await mkdir(dirname(output), { recursive: true });
  // Never overwrite an existing feed. Failed verification leaves it untouched.
  await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`, { flag: "wx" });
  console.log(`Verified updater manifest: ${output}`);
}
