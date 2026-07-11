import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";

const script = join(dirname(fileURLToPath(import.meta.url)), "sync-mirror.sh");
const tempDirs = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("mirror identity staging", () => {
  it("reuses identical immutable identity bytes and rejects a rerun with a new signature", () => {
    const root = mkdtempSync(join(tmpdir(), "cam-mirror-identity-"));
    tempDirs.push(root);
    const dist = join(root, "dist");
    const bin = join(root, "bin");
    const store = join(root, "s3");
    mkdirSync(dist);
    mkdirSync(bin);
    mkdirSync(store);
    writeFileSync(join(dist, "app.bin"), "artifact");
    writeFileSync(join(dist, "release-identity.json"), '{"schema":1,"version":"1.2.3"}\n');
    writeFileSync(join(dist, "release-identity.json.sig"), "first-signature\n");
    writeFileSync(
      join(dist, "latest.json"),
      JSON.stringify({
        version: "1.2.3",
        platforms: { test: { url: "https://github.test/app.bin", signature: "artifact-sig" } },
      }),
    );

    const fakeAws = join(bin, "aws");
    writeFileSync(
      fakeAws,
      `#!/usr/bin/env bash
set -euo pipefail
if [ "$1 $2" = "s3api head-object" ]; then
  shift 2; bucket=""; key=""
  while [ "$#" -gt 0 ]; do
    case "$1" in --bucket) bucket="$2"; shift 2;; --key) key="$2"; shift 2;; *) shift;; esac
  done
  [ -f "$FAKE_S3/$bucket/$key" ] && { echo '{}'; exit 0; }
  echo 'An error occurred (404) when calling HeadObject' >&2; exit 255
fi
if [ "$1 $2" = "s3 cp" ]; then
  src="$3"; dst="$4"
  if [[ "$src" == s3://* ]]; then
    remote="\${src#s3://}"; cp "$FAKE_S3/$remote" "$dst"
  else
    remote="\${dst#s3://}"; mkdir -p "$(dirname "$FAKE_S3/$remote")"; cp "$src" "$FAKE_S3/$remote"
    echo "$remote" >> "$FAKE_AWS_LOG"
  fi
  exit 0
fi
echo "unsupported fake aws invocation: $*" >&2; exit 2
`,
    );
    chmodSync(fakeAws, 0o755);

    const env = {
      ...process.env,
      PATH: `${bin}${delimiter}${process.env.PATH}`,
      FAKE_S3: store,
      FAKE_AWS_LOG: join(root, "aws.log"),
      MIRROR_PHASE: "stage",
      MIRROR_BASE_URL: "https://mirror.test/manager",
      MANAGER_R2_S3_ENDPOINT: "https://r2.test",
      MANAGER_R2_BUCKET: "bucket",
      MANAGER_R2_ACCESS_KEY_ID: "test-ak",
      MANAGER_R2_SECRET_ACCESS_KEY: "test-sk",
    };
    const run = () => spawnSync("bash", [script, dist], { encoding: "utf8", env });

    const first = run();
    expect(first.status, first.stderr).toBe(0);
    const second = run();
    expect(second.status, second.stderr).toBe(0);
    expect(second.stdout).toContain("identical immutable object already staged");
    const uploads = readFileSync(env.FAKE_AWS_LOG, "utf8").trim().split("\n");
    expect(uploads.filter((key) => key.endsWith("/release-identity.json"))).toHaveLength(1);
    expect(uploads.filter((key) => key.endsWith("/release-identity.json.sig"))).toHaveLength(1);

    writeFileSync(join(dist, "release-identity.json.sig"), "different-rerun-signature\n");
    const changed = run();
    expect(changed.status).not.toBe(0);
    expect(changed.stderr).toContain("refusing to overwrite byte-different immutable identity object");
  });
});
