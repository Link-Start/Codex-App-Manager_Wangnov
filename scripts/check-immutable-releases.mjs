#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const API_VERSION = "2026-03-10";
const REPOSITORY_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

export function assertImmutableReleasesEnabled(settings) {
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
    throw new Error("GitHub Immutable Releases response must be an object");
  }
  if (settings.enabled !== true) {
    throw new Error(
      "GitHub Immutable Releases are disabled; enable them before starting a release",
    );
  }
  return { enabled: true };
}

export function verifyImmutableReleases({
  repository = process.env.GITHUB_REPOSITORY || "",
  token = process.env.GH_TOKEN || "",
  runner = spawnSync,
} = {}) {
  if (!REPOSITORY_PATTERN.test(repository)) {
    throw new Error("GITHUB_REPOSITORY must be an owner/repository name");
  }
  if (!token.trim()) {
    throw new Error(
      "IMMUTABLE_RELEASES_READ_TOKEN is missing; release preflight cannot verify repository settings",
    );
  }

  const result = runner(
    "gh",
    [
      "api",
      "-H",
      `X-GitHub-Api-Version: ${API_VERSION}`,
      `repos/${repository}/immutable-releases`,
    ],
    {
      encoding: "utf8",
      env: { ...process.env, GH_TOKEN: token },
    },
  );
  if (result.error || result.status !== 0) {
    const detail = String(result.error?.message || result.stderr || result.stdout || "unknown error")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 500);
    throw new Error(
      `could not verify GitHub Immutable Releases with the read-only token: ${detail}`,
    );
  }

  let settings;
  try {
    settings = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`GitHub Immutable Releases response was not valid JSON: ${error.message}`);
  }
  return assertImmutableReleasesEnabled(settings);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    verifyImmutableReleases();
    console.log("GitHub Immutable Releases are enabled");
  } catch (error) {
    console.error(`::error::${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
