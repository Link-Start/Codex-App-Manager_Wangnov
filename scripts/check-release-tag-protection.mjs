#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const API_VERSION = "2026-03-10";
const REPOSITORY_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const RELEASE_TAG_PATTERN =
  /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/;
const SHA_PATTERN = /^[0-9a-f]{40}$/;
const REQUIRED_REF_PATTERN = "refs/tags/v*";

function parseJson(stdout, label) {
  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`${label} was not valid JSON: ${error.message}`);
  }
}

function errorDetail(result) {
  return String(
    result.error?.message || result.stderr || result.stdout || "unknown error",
  )
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 500);
}

export function assertReleaseTagRuleset(rulesets) {
  if (!Array.isArray(rulesets)) {
    throw new Error("GitHub tag rulesets response must be an array");
  }

  const matching = rulesets.find((ruleset) => {
    const includes = ruleset?.conditions?.ref_name?.include;
    const excludes = ruleset?.conditions?.ref_name?.exclude;
    const ruleTypes = new Set(
      Array.isArray(ruleset?.rules)
        ? ruleset.rules.map((rule) => rule?.type)
        : [],
    );
    const bypassActors = ruleset?.bypass_actors;
    return (
      ruleset?.target === "tag" &&
      ruleset?.enforcement === "active" &&
      Array.isArray(includes) &&
      includes.includes(REQUIRED_REF_PATTERN) &&
      Array.isArray(excludes) &&
      excludes.length === 0 &&
      ruleTypes.has("update") &&
      ruleTypes.has("deletion") &&
      (!Array.isArray(bypassActors) || bypassActors.length === 0)
    );
  });

  if (!matching) {
    throw new Error(
      `an active tag ruleset must protect ${REQUIRED_REF_PATTERN} from update and deletion without exclusions or visible bypass actors`,
    );
  }

  return { id: matching.id, name: matching.name };
}

export function assertReleaseTagCommit(actualSha, expectedSha) {
  if (!SHA_PATTERN.test(expectedSha)) {
    throw new Error(
      "expected release source SHA must be a lowercase 40-character commit SHA",
    );
  }
  if (!SHA_PATTERN.test(actualSha)) {
    throw new Error("live release tag did not peel to a commit SHA");
  }
  if (actualSha !== expectedSha) {
    throw new Error(
      `release tag moved after validation: expected ${expectedSha}, found ${actualSha}`,
    );
  }
  return { sha: actualSha };
}

export function verifyReleaseTagProtection({
  repository = process.env.GITHUB_REPOSITORY || "",
  releaseTag = process.argv[2] || process.env.RELEASE_TAG || "",
  expectedSha = process.argv[3] || process.env.RELEASE_SOURCE_SHA || "",
  token = process.env.GH_TOKEN || "",
  runner = spawnSync,
} = {}) {
  if (!REPOSITORY_PATTERN.test(repository)) {
    throw new Error("GITHUB_REPOSITORY must be an owner/repository name");
  }
  if (!RELEASE_TAG_PATTERN.test(releaseTag)) {
    throw new Error("release tag must be a semantic vX.Y.Z tag");
  }
  if (!SHA_PATTERN.test(expectedSha)) {
    throw new Error(
      "expected release source SHA must be a lowercase 40-character commit SHA",
    );
  }
  if (!token.trim()) {
    throw new Error(
      "IMMUTABLE_RELEASES_READ_TOKEN is missing; release cannot verify tag protection",
    );
  }

  const api = (endpoint) => {
    const result = runner(
      "gh",
      ["api", "-H", `X-GitHub-Api-Version: ${API_VERSION}`, endpoint],
      {
        encoding: "utf8",
        env: { ...process.env, GH_TOKEN: token },
      },
    );
    if (result.error || result.status !== 0) {
      throw new Error(
        `could not verify GitHub release tag protection: ${errorDetail(result)}`,
      );
    }
    return parseJson(result.stdout, `GitHub API response for ${endpoint}`);
  };

  const summaries = api(
    `repos/${repository}/rulesets?targets=tag&per_page=100`,
  );
  if (!Array.isArray(summaries)) {
    throw new Error("GitHub tag rulesets response must be an array");
  }
  const details = summaries
    .filter(
      (ruleset) =>
        ruleset?.target === "tag" && ruleset?.enforcement === "active",
    )
    .map((ruleset) => api(`repos/${repository}/rulesets/${ruleset.id}`));
  const ruleset = assertReleaseTagRuleset(details);

  let object = api(`repos/${repository}/git/ref/tags/${releaseTag}`).object;
  for (let depth = 0; object?.type === "tag" && depth < 5; depth += 1) {
    object = api(`repos/${repository}/git/tags/${object.sha}`).object;
  }
  if (object?.type !== "commit") {
    throw new Error("live release tag did not peel to a commit");
  }
  const commit = assertReleaseTagCommit(object.sha, expectedSha);

  return { commit, ruleset };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const result = verifyReleaseTagProtection();
    console.log(
      `Release tag protection verified by ${result.ruleset.name || result.ruleset.id}: ${result.commit.sha}`,
    );
  } catch (error) {
    console.error(
      `::error::${error instanceof Error ? error.message : String(error)}`,
    );
    process.exitCode = 1;
  }
}
