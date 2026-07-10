import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  assertImmutableReleasesEnabled,
  verifyImmutableReleases,
} from "./check-immutable-releases.mjs";

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
    const stage = releaseJob.indexOf("- name: Stage CDN mirror candidate");
    const mirrorVerify = releaseJob.indexOf(
      "- name: Verify staged CDN mirror before immutable publication",
    );
    const upload = releaseJob.indexOf("- name: Upload GitHub Release draft");
    const publish = releaseJob.indexOf("- name: Publish GitHub Release");
    const publishedVerify = releaseJob.indexOf(
      "- name: Verify published immutable Release and asset digests",
    );
    const attest = releaseJob.indexOf("- name: Attest build provenance");
    const promote = releaseJob.indexOf("- name: Promote CDN mirror latest");
    const winget = releaseJob.indexOf("- name: Trigger winget submission");
    const summary = releaseJob.indexOf("- name: Write release summary");
    expect(localVerify).toBeGreaterThan(-1);
    expect(stage).toBeGreaterThan(localVerify);
    expect(mirrorVerify).toBeGreaterThan(stage);
    expect(upload).toBeGreaterThan(mirrorVerify);
    expect(upload).toBeGreaterThan(-1);
    expect(publish).toBeGreaterThan(upload);
    expect(publishedVerify).toBeGreaterThan(publish);
    expect(attest).toBeGreaterThan(publishedVerify);
    expect(promote).toBeGreaterThan(attest);

    const uploadStep = releaseJob.slice(upload, publish);
    const publishStep = releaseJob.slice(publish, publishedVerify);
    const verifyStep = releaseJob.slice(publishedVerify, attest);
    const attestStep = releaseJob.slice(attest, promote);
    const promoteStep = releaseJob.slice(promote, winget);
    const wingetStep = releaseJob.slice(winget, summary);
    const localVerifyStep = releaseJob.slice(localVerify, stage);
    const mirrorVerifyStep = releaseJob.slice(mirrorVerify, upload);
    expect(localVerifyStep).toContain("node scripts/verify-release-artifacts.mjs");
    expect(mirrorVerifyStep).toContain("MIRROR_PHASE: verify");
    expect(mirrorVerifyStep).toContain("bash scripts/sync-mirror.sh dist");
    expect(uploadStep).toContain("draft: true");
    expect(uploadStep).toContain("prerelease: ${{ contains(env.RELEASE_TAG, '-') }}");
    expect(uploadStep).toContain("files: |");
    expect(publishStep).not.toMatch(/^\s+draft:/m);
    expect(publishStep).not.toContain("files: |");
    expect(verifyStep).toContain("node scripts/check-release-reuse.mjs");
    expect(verifyStep).toContain("did not become immutable with canonical asset digests");
    expect(attestStep).toContain(
      "steps.publish_release.outcome == 'success' || steps.release_source.outputs.existing == 'true'",
    );
    expect(attestStep).toContain("actions/attest-build-provenance@");
    expect(attestStep).not.toContain("continue-on-error: true");
    expect(promoteStep).toContain("steps.attest.outcome == 'success'");
    expect(wingetStep).toContain("steps.attest.outcome == 'success'");
    expect(releaseJob.slice(0, upload)).toContain(
      "rm -f dist/latest.mirror.json dist/latest.json",
    );
  });

  it("attests only immutable Release bytes on a failed-job reuse", () => {
    const source = releaseJob.indexOf("- name: Resolve immutable release artifact source");
    const validate = releaseJob.indexOf("- name: Validate final release artifacts");
    const attest = releaseJob.indexOf("- name: Attest build provenance");
    const promote = releaseJob.indexOf("- name: Promote CDN mirror latest");
    const sourceStep = releaseJob.slice(source, validate);
    const attestStep = releaseJob.slice(attest, promote);

    expect(sourceStep).toContain("gh release download");
    expect(sourceStep).toContain("--pattern 'CodexAppManager*'");
    expect(sourceStep).toContain("--pattern 'latest.json'");
    expect(sourceStep).toContain('actual_digest="sha256:$(sha256sum "$file"');
    expect(sourceStep).toContain('if [[ "$actual_digest" != "$expected_digest" ]]');
    expect(attestStep).toContain("dist/*");
    expect(attestStep).toContain("latest.json");
    expect(attestStep).not.toContain("SHA256SUMS");
    expect(attestStep).not.toContain("release-assets/*");
  });
});
