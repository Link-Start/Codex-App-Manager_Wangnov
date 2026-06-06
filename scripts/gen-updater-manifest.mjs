#!/usr/bin/env node
// Build the Tauri updater manifest (latest.json) from collected release
// artifacts. Each macOS updater tarball is renamed with its arch during the
// CI "Collect artifacts" step and carries a sibling .sig; we read those sigs
// and point the urls at the GitHub release download path. The manifest is
// served as a release asset, matching the updater endpoints in tauri.conf.json.
//
// Usage: node scripts/gen-updater-manifest.mjs <tag> <artifacts-dir>
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [, , tag, dir] = process.argv;
if (!tag || !dir) {
  console.error("usage: gen-updater-manifest.mjs <tag> <artifacts-dir>");
  process.exit(2);
}
const version = tag.replace(/^v/, "");
const REPO = "Wangnov/Codex-App-Manager";
const downloadUrl = (file) =>
  `https://github.com/${REPO}/releases/download/${tag}/${encodeURIComponent(file)}`;

const files = readdirSync(dir);
const findSig = (re) => files.find((f) => re.test(f) && f.endsWith(".sig"));

// Tauri updater platform keys → how to spot that platform's signed bundle.
const MATCHERS = [
  ["darwin-aarch64", /aarch64.*\.app\.tar\.gz\.sig$/],
  ["darwin-x86_64", /x86_64.*\.app\.tar\.gz\.sig$/],
  ["windows-x86_64", /(?:x64|x86_64).*-setup\.(?:exe|nsis\.zip)\.sig$/],
];

const platforms = {};
for (const [key, re] of MATCHERS) {
  const sig = findSig(re);
  if (!sig) {
    console.warn(`[manifest] no signed artifact for ${key} — skipping`);
    continue;
  }
  const bundle = sig.replace(/\.sig$/, "");
  platforms[key] = {
    signature: readFileSync(join(dir, sig), "utf8").trim(),
    url: downloadUrl(bundle),
  };
}

if (Object.keys(platforms).length === 0) {
  console.error("[manifest] no platforms resolved — check artifact globs");
  process.exit(1);
}

const manifest = {
  version,
  notes: `Codex App Manager ${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
};
writeFileSync("latest.json", JSON.stringify(manifest, null, 2));
console.log("wrote latest.json:\n" + JSON.stringify(manifest, null, 2));
