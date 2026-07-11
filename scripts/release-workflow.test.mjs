import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  assertImmutableReleasesEnabled,
  verifyImmutableReleases,
} from "./check-immutable-releases.mjs";
import { requiredReleaseAssetNames } from "./check-release-reuse.mjs";
import {
  assertLocalReleaseArtifactNames,
  assertReleaseSourceVersions,
} from "./check-release-version.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const workflow = await readFile(join(repoRoot, ".github/workflows/release.yml"), "utf8");
const mirrorRelease = await readFile(join(repoRoot, "scripts/mirror-release.mjs"), "utf8");
const releaseJob = workflow.slice(workflow.indexOf("  release:\n"));

describe("release workflow recovery invariants", () => {
  it("queues every release run instead of replacing an older pending tag", () => {
    expect(workflow).toMatch(
      /concurrency:\n\s+group: release-latest-\$\{\{ github\.repository \}\}\n\s+cancel-in-progress: false\n\s+queue: max/,
    );
  });

  it("rejects every manual dispatch outside the full default-branch ref before checkout or secrets", () => {
    const dispatchInputs = workflow.slice(
      workflow.indexOf("  workflow_dispatch:\n"),
      workflow.indexOf("\npermissions:\n"),
    );
    const reject = workflow.slice(
      workflow.indexOf("  reject_untrusted_target_dispatch:\n"),
      workflow.indexOf("  preflight:\n"),
    );
    const preflight = workflow.slice(
      workflow.indexOf("  preflight:\n"),
      workflow.indexOf("  prepare:\n"),
    );
    const prepare = workflow.slice(
      workflow.indexOf("  prepare:\n"),
      workflow.indexOf("  build:\n"),
    );

    expect(reject).toContain("github.event_name == 'workflow_dispatch'");
    expect(reject).toContain(
      "github.ref != format('refs/heads/{0}', github.event.repository.default_branch)",
    );
    expect(reject).not.toContain("inputs.target_tag != ''");
    expect(reject).not.toContain("actions/checkout@");
    expect(reject).not.toContain("environment:");
    expect(reject).not.toContain("${{ secrets.");
    expect(dispatchInputs).toMatch(/target_tag:\n\s+description:.*\n\s+required: true/);

    expect(preflight).toContain(
      "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)",
    );
    expect(preflight).toContain("workflow_dispatch requires a non-empty target_tag");
    expect(preflight).not.toContain("actions/checkout@");
    expect(preflight).not.toContain("environment:");
    expect(preflight).not.toContain("${{ secrets.");
    expect(prepare).toContain("needs: preflight");
    expect(prepare).toContain("needs.preflight.result == 'success'");
  });

  it("binds the release tag to all six source versions and exact local artifact names", async () => {
    const packageJson = JSON.parse(await readFile(join(repoRoot, "package.json"), "utf8"));
    const releaseTag = `v${packageJson.version}`;
    const source = assertReleaseSourceVersions(releaseTag, repoRoot);
    expect(source.entries).toHaveLength(6);
    expect(() => assertReleaseSourceVersions("v9.9.9", repoRoot)).toThrow(
      "mismatched source versions",
    );

    const artifactsDir = await mkdtemp(join(tmpdir(), "cam-release-artifacts-"));
    try {
      const expected = requiredReleaseAssetNames(releaseTag).filter(
        (name) => name !== "latest.json",
      );
      await Promise.all(expected.map((name) => writeFile(join(artifactsDir, name), "x")));
      expect(assertLocalReleaseArtifactNames(releaseTag, artifactsDir).expected).toEqual(
        expected,
      );

      await writeFile(join(artifactsDir, "CodexAppManager_9.9.9_x64-setup.exe"), "x");
      expect(() => assertLocalReleaseArtifactNames(releaseTag, artifactsDir)).toThrow(
        "installer artifacts for another version",
      );
    } finally {
      await rm(artifactsDir, { force: true, recursive: true });
    }

    const prepare = workflow.slice(
      workflow.indexOf("  prepare:\n"),
      workflow.indexOf("  build:\n"),
    );
    expect(prepare).toContain('ref: ${{ env.RELEASE_TAG }}');
    expect(prepare).toContain(
      'node scripts/check-release-version.mjs source "$RELEASE_TAG" release-source',
    );
    expect(releaseJob).toContain(
      'node scripts/check-release-version.mjs artifacts "$RELEASE_TAG" dist',
    );
    const manifestStep = releaseJob.slice(
      releaseJob.indexOf("- name: Generate updater manifest (latest.json)"),
      releaseJob.indexOf(
        "- name: Verify local updater signatures before immutable publication",
      ),
    );
    expect(manifestStep).toContain("node scripts/validate-release-manifest.mjs");
    expect(manifestStep).not.toContain('if [[ "$RELEASE_TAG" != *-* ]]');
  });

  it("enforces the single-writer boundary for the unconditional IHEP follower", () => {
    expect(workflow).toContain(
      "correctness boundary for the unconditional IHEP follower",
    );
    expect(releaseJob).toContain("Enforce legacy mirror credential revocation");
    expect(releaseJob).toContain("MANAGER_R2_PROMOTION_ACCESS_KEY_ID");
    expect(releaseJob).toContain("MANAGER_IHEP_S3_PROMOTION_ACCESS_KEY_ID");
    expect(releaseJob).toContain(
      "MANAGER_IHEP_S3_ENDPOINT: ${{ vars.MANAGER_IHEP_S3_ENDPOINT }}",
    );
    expect(releaseJob).toContain(
      "MANAGER_IHEP_S3_BUCKET: ${{ vars.MANAGER_IHEP_S3_BUCKET }}",
    );
    expect(releaseJob).not.toContain(
      "MANAGER_IHEP_S3_ENDPOINT: ${{ secrets.MANAGER_IHEP_S3_ENDPOINT }}",
    );
    expect(mirrorRelease).toContain(
      "followerCommit = await ihep.putLatestUnconditional(candidatePath, promotionToken)",
    );
    expect(mirrorRelease).not.toContain("await ihep.putLatestConditional(");
  });

  it("refreshes immutable Release state inside the release job on failed-job reruns", () => {
    expect(releaseJob).toContain("id: live_release");
    expect(releaseJob).toContain(
      "if: ${{ steps.live_release.outputs.release_reusable != 'true' }}",
    );
    expect(releaseJob).toContain(
      "RELEASE_ASSET_DIGESTS: ${{ steps.live_release.outputs.release_asset_digests }}",
    );
    expect(releaseJob).toContain(
      'if [[ "${{ steps.live_release.outputs.release_reusable }}" == "true" ]]; then',
    );
    expect(releaseJob).not.toContain(
      "if: ${{ needs.prepare.outputs.release_reusable != 'true' }}",
    );
  });

  it("uses the target tag updater trust root without executing historical scripts", () => {
    const refresh = releaseJob.indexOf("- name: Refresh immutable release state");
    const resolveTrust = releaseJob.indexOf(
      "- name: Resolve updater trust root for release tag",
    );
    const download = releaseJob.indexOf("- name: Download canonical build artifacts");
    const localVerify = releaseJob.indexOf(
      "- name: Verify local updater signatures before immutable publication",
    );
    const stage = releaseJob.indexOf("- name: Stage CDN mirror candidate");
    const trustStep = releaseJob.slice(resolveTrust, download);
    const localVerifyStep = releaseJob.slice(localVerify, stage);

    expect(resolveTrust).toBeGreaterThan(refresh);
    expect(download).toBeGreaterThan(resolveTrust);
    expect(trustStep).toContain("gh api --method GET");
    expect(trustStep).toContain("application/vnd.github.raw+json");
    expect(trustStep).toContain('-f ref="$RELEASE_TAG"');
    expect(trustStep).toContain("RELEASE_TAURI_CONFIG=");
    expect(trustStep).toContain("MIRROR_UPDATER_PUBLIC_KEY=");
    expect(trustStep).toContain('>> "$GITHUB_ENV"');
    expect(localVerifyStep).toContain('"$RELEASE_TAURI_CONFIG"');
    expect(localVerifyStep).not.toContain("src-tauri/tauri.conf.json");
    expect(mirrorRelease).toContain("process.env.MIRROR_UPDATER_PUBLIC_KEY ||");
  });

  it("fails closed when immutable settings are disabled or cannot be queried", () => {
    expect(assertImmutableReleasesEnabled({ enabled: true })).toEqual({ enabled: true });
    expect(() => assertImmutableReleasesEnabled({ enabled: false })).toThrow(
      "GitHub Immutable Releases are disabled",
    );
    expect(() =>
      verifyImmutableReleases({
        repository: "owner/repo",
        token: "read-only-token",
        runner: () => ({ status: 1, stderr: "HTTP 403", stdout: "" }),
      }),
    ).toThrow("could not verify GitHub Immutable Releases");

    const prepare = workflow.slice(
      workflow.indexOf("  prepare:\n"),
      workflow.indexOf("  build:\n"),
    );
    expect(prepare).toContain("environment: release");
    expect(prepare).toContain("GH_TOKEN: ${{ secrets.IMMUTABLE_RELEASES_READ_TOKEN }}");
    expect(prepare).toContain("run: node scripts/check-immutable-releases.mjs");
    expect(releaseJob).toContain("run: node scripts/check-immutable-releases.mjs");
  });

  it("uploads stable and prerelease assets to a draft before publishing", () => {
    const localVerify = releaseJob.indexOf(
      "- name: Verify local updater signatures before immutable publication",
    );
    const attest = releaseJob.indexOf("- name: Attest fresh build provenance");
    const verifyExisting = releaseJob.indexOf(
      "- name: Verify existing immutable release provenance",
    );
    const provenance = releaseJob.indexOf("- name: Resolve provenance gate");
    const stage = releaseJob.indexOf("- name: Stage CDN mirror candidate");
    const mirrorVerify = releaseJob.indexOf(
      "- name: Verify staged CDN mirror before immutable publication",
    );
    const upload = releaseJob.indexOf("- name: Upload GitHub Release draft");
    const publish = releaseJob.indexOf("- name: Publish GitHub Release");
    const publishedVerify = releaseJob.indexOf(
      "- name: Verify published immutable Release and asset digests",
    );
    const promote = releaseJob.indexOf("- name: Promote CDN mirror latest");
    const winget = releaseJob.indexOf("- name: Trigger winget submission");
    const summary = releaseJob.indexOf("- name: Write release summary");
    expect(localVerify).toBeGreaterThan(-1);
    expect(attest).toBeGreaterThan(localVerify);
    expect(verifyExisting).toBeGreaterThan(attest);
    expect(provenance).toBeGreaterThan(verifyExisting);
    expect(stage).toBeGreaterThan(provenance);
    expect(mirrorVerify).toBeGreaterThan(stage);
    expect(upload).toBeGreaterThan(mirrorVerify);
    expect(upload).toBeGreaterThan(-1);
    expect(publish).toBeGreaterThan(upload);
    expect(publishedVerify).toBeGreaterThan(publish);
    expect(promote).toBeGreaterThan(publishedVerify);

    const uploadStep = releaseJob.slice(upload, publish);
    const publishStep = releaseJob.slice(publish, publishedVerify);
    const verifyStep = releaseJob.slice(publishedVerify, promote);
    const attestStep = releaseJob.slice(attest, verifyExisting);
    const existingStep = releaseJob.slice(verifyExisting, provenance);
    const provenanceStep = releaseJob.slice(provenance, stage);
    const promoteStep = releaseJob.slice(promote, winget);
    const wingetStep = releaseJob.slice(winget, summary);
    const localVerifyStep = releaseJob.slice(localVerify, stage);
    const mirrorVerifyStep = releaseJob.slice(mirrorVerify, upload);
    expect(localVerifyStep).toContain("node scripts/verify-release-artifacts.mjs");
    expect(mirrorVerifyStep).toContain("MIRROR_PHASE: verify");
    expect(mirrorVerifyStep).toContain("bash scripts/sync-mirror.sh dist");
    expect(uploadStep).toContain("draft: true");
    expect(uploadStep).toContain("steps.provenance.outputs.ready == 'true'");
    expect(uploadStep).toContain("prerelease: ${{ contains(env.RELEASE_TAG, '-') }}");
    expect(uploadStep).toContain("files: |");
    expect(publishStep).not.toMatch(/^\s+draft:/m);
    expect(publishStep).not.toContain("files: |");
    expect(publishStep).toContain("steps.provenance.outputs.ready == 'true'");
    expect(verifyStep).toContain("node scripts/check-release-reuse.mjs");
    expect(verifyStep).toContain("did not become immutable with canonical asset digests");
    expect(verifyStep).toContain("published immutable digest does not match attested local bytes");
    expect(attestStep).toContain("steps.release_source.outputs.existing != 'true'");
    expect(attestStep).toContain("actions/attest-build-provenance@");
    expect(attestStep).not.toContain("continue-on-error: true");
    expect(existingStep).toContain("gh attestation verify");
    expect(existingStep).toContain('--signer-workflow "$signer_workflow"');
    expect(existingStep).toContain('--source-ref "refs/tags/$RELEASE_TAG"');
    expect(existingStep).toContain('--source-digest "$RELEASE_SOURCE_SHA"');
    expect(provenanceStep).toContain('echo "ready=true" >> "$GITHUB_OUTPUT"');
    expect(promoteStep).toContain("steps.provenance.outputs.ready == 'true'");
    expect(promoteStep).not.toContain("steps.attest_fresh.outcome");
    expect(wingetStep).toContain("steps.provenance.outputs.ready == 'true'");
    expect(wingetStep).not.toContain("steps.attest_fresh.outcome");
    expect(releaseJob.slice(0, upload)).toContain(
      "rm -f dist/latest.mirror.json dist/latest.json",
    );
  });

  it("verifies existing immutable provenance without minting a rerun attestation", () => {
    const source = releaseJob.indexOf("- name: Resolve immutable release artifact source");
    const validate = releaseJob.indexOf("- name: Validate final release artifacts");
    const attest = releaseJob.indexOf("- name: Attest fresh build provenance");
    const verifyExisting = releaseJob.indexOf(
      "- name: Verify existing immutable release provenance",
    );
    const provenance = releaseJob.indexOf("- name: Resolve provenance gate");
    const sourceStep = releaseJob.slice(source, validate);
    const attestStep = releaseJob.slice(attest, verifyExisting);
    const existingStep = releaseJob.slice(verifyExisting, provenance);

    expect(sourceStep).toContain("gh release download");
    expect(sourceStep).toContain("--pattern 'CodexAppManager*'");
    expect(sourceStep).toContain("--pattern 'latest.json'");
    expect(sourceStep).toContain('actual_digest="sha256:$(sha256sum "$file"');
    expect(sourceStep).toContain('if [[ "$actual_digest" != "$expected_digest" ]]');
    expect(attestStep).toContain("steps.release_source.outputs.existing != 'true'");
    expect(attestStep).not.toContain("steps.release_source.outputs.existing == 'true'");
    expect(existingStep).toContain("steps.release_source.outputs.existing == 'true'");
    expect(existingStep).toContain("for file in dist/* latest.json");
    expect(existingStep).toContain("gh attestation verify");
    expect(existingStep).toContain("--deny-self-hosted-runners");
  });
});
