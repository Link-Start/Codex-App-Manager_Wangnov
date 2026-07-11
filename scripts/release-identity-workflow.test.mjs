import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

describe("release identity workflow wiring", () => {
  it("publishes one latest.json asset while retaining versioned identity assets", () => {
    const workflow = readFileSync(join(root, ".github", "workflows", "release.yml"), "utf8");
    expect(workflow).toContain("cp release-identity.json release-identity.json.sig dist/");
    expect(workflow).not.toMatch(/cp latest\.json release-identity\.json/);
    expect(workflow.match(/rm -f dist\/latest\.json dist\/latest\.mirror\.json/g)).toHaveLength(2);
    expect(workflow).toMatch(/files:\s*\|[\s\S]*?dist\/\*[\s\S]*?\n\s+latest\.json/);
  });
});
