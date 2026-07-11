import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";

const script = join(dirname(fileURLToPath(import.meta.url)), "gen-updater-manifest.mjs");
const tempDirs = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("release identity generation", () => {
  it("is byte-stable across reruns and excludes the manifest clock", () => {
    const root = mkdtempSync(join(tmpdir(), "cam-release-identity-"));
    tempDirs.push(root);
    mkdirSync(join(root, "dist"));
    mkdirSync(join(root, "docs", "releases"), { recursive: true });
    writeFileSync(join(root, "docs", "releases", "v1.2.3.md"), "Reviewed notes\n");

    const fixtures = [
      ["CodexAppManager_aarch64.app.tar.gz", "mac-arm"],
      ["CodexAppManager_x86_64.app.tar.gz", "mac-x64"],
      ["CodexAppManager_1.2.3_x64-setup.exe", "win-x64"],
      ["CodexAppManager_1.2.3_arm64-setup.exe", "win-arm"],
    ];
    for (const [name, bytes] of fixtures) {
      writeFileSync(join(root, "dist", name), bytes);
      writeFileSync(join(root, "dist", `${name}.sig`), `signature-${name}`);
    }

    const run = () => {
      const result = spawnSync(process.execPath, [script, "v1.2.3", "dist"], {
        cwd: root,
        encoding: "utf8",
      });
      expect(result.status, result.stderr).toBe(0);
      return readFileSync(join(root, "release-identity.json"));
    };
    const first = run();
    const second = run();
    expect(second.equals(first)).toBe(true);

    const identity = JSON.parse(first.toString("utf8"));
    expect(identity).not.toHaveProperty("pub_date");
    expect(identity.notes_sha256).toBe(
      createHash("sha256").update("Reviewed notes", "utf8").digest("hex"),
    );
    expect(identity.platforms["windows-x86_64"]).toMatchObject({
      artifact: "CodexAppManager_1.2.3_x64-setup.exe",
      sha256: createHash("sha256").update("win-x64").digest("hex"),
    });
  });
});
