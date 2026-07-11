#!/usr/bin/env node

import { readFile } from "node:fs/promises";

import { assertCandidateMatchesRelease } from "./mirror-release.mjs";

const [, , releaseTag, candidatePath, artifactManifestPath] = process.argv;
if (!releaseTag || !candidatePath || !artifactManifestPath) {
  console.error(
    "usage: validate-release-manifest.mjs <vX.Y.Z> <candidate-latest.json> <artifact-derived-latest.json>",
  );
  process.exit(2);
}

const readManifest = async (path, label) => {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`cannot read ${label}: ${detail}`);
  }
};

try {
  const [candidate, artifactManifest] = await Promise.all([
    readManifest(candidatePath, "candidate latest.json"),
    readManifest(artifactManifestPath, "artifact-derived latest.json"),
  ]);
  const result = assertCandidateMatchesRelease(candidate, artifactManifest, releaseTag);
  console.log(
    `[manifest] candidate is bound to ${releaseTag} (${result.platformCount} updater platforms)`,
  );
} catch (error) {
  const detail = error instanceof Error ? error.message : String(error);
  console.error(`::error::[manifest] ${detail}`);
  process.exitCode = 1;
}
