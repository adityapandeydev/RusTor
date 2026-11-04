import { createHash } from "node:crypto";
import { readdir, readFile, stat } from "node:fs/promises";
import { join } from "node:path";

export function platformPlan(target) {
  const plans = {
    "x86_64-pc-windows-msvc": { bundles: ["nsis"], arch: "x64" },
    "x86_64-unknown-linux-gnu": { bundles: ["appimage", "deb"], arch: "x64" },
    "aarch64-apple-darwin": { bundles: ["dmg"], arch: "arm64" },
    "x86_64-apple-darwin": { bundles: ["dmg"], arch: "x64" },
  };
  if (!Object.hasOwn(plans, target)) throw new Error(`Unsupported packaging target: ${target}`);
  return plans[target];
}

export function renderNotices(rust, frontend) {
  if (!Array.isArray(rust.licenses) || rust.licenses.length === 0 || !Array.isArray(frontend))
    throw new Error("Missing dependency license reports");
  for (const name of ["react", "react-dom", "scheduler", "lucide-react", "@tauri-apps/api", "@fontsource-variable/manrope"])
    if (!frontend.some((row) => row.name === name)) throw new Error(`Missing bundled notice: ${name}`);
  const sections = ["RusTor third-party dependency licenses", "Project and Torlink credits and notices are in the accompanying README.md.\nRusTor's license is in LICENSE."];
  for (const license of rust.licenses) {
    if (!license.text?.trim() || !license.used_by?.length) throw new Error("Incomplete Rust license text or package attribution");
    const packages = license.used_by.map(({ crate }) => {
      let source = `https://crates.io/api/v1/crates/${crate.name}/${crate.version}/download`;
      if (crate.source?.startsWith("git+")) {
        const git = /^git\+(https:\/\/github\.com\/[^/?#]+\/[^/?#]+)(?:\?[^#]*)?#([0-9a-f]{40})$/.exec(crate.source);
        if (!git) throw new Error(`Unrecognized pinned source for ${crate.name}`);
        source = `${git[1].replace(/\.git$/, "")}/archive/${git[2]}.tar.gz`;
      }
      return `${crate.name} ${crate.version} — ${source}`;
    }).sort();
    sections.push(`Rust: ${license.id}\n${packages.join("\n")}\n\n${license.text}`);
  }
  const accepted = new Set(["MIT", "ISC", "OFL-1.1", "Apache-2.0 OR MIT"]);
  for (const row of [...frontend].sort((a, b) => a.name.localeCompare(b.name))) {
    if (!accepted.has(row.identifier) || !row.text?.trim()) throw new Error(`Unreviewed or missing frontend license: ${row.name}`);
    sections.push(`Frontend: ${row.name} ${row.version} (${row.identifier})\n\n${row.text}`);
  }
  return sections.join("\n\n==============================\n\n") + "\n";
}

export async function findBundles(directory, formats, version, since) {
  const extensions = { nsis: ".exe", appimage: ".AppImage", deb: ".deb", dmg: ".dmg" };
  const files = [];
  async function visit(path) {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const file = join(path, entry.name);
      if (entry.isDirectory()) await visit(file);
      else if (entry.isFile() && entry.name.includes(`_${version}_`) && (await stat(file)).mtimeMs >= since)
        files.push(file);
    }
  }
  await visit(directory);
  return formats.map((format) => {
    const matching = files.filter((file) => file.endsWith(extensions[format]));
    if (matching.length !== 1) throw new Error(`Expected one newly built ${format} package, found ${matching.length}`);
    return matching[0];
  });
}

export async function checksumManifest(directory) {
  const names = (await readdir(directory, { withFileTypes: true }))
    .filter((entry) => entry.isFile() && entry.name !== "SHA256SUMS")
    .map((entry) => entry.name).sort();
  return (await Promise.all(names.map(async (name) => {
    if (/[\r\n]/.test(name)) throw new Error("Invalid artifact filename");
    return `${createHash("sha256").update(await readFile(join(directory, name))).digest("hex")}  ${name}\n`;
  }))).join("");
}
