import { describe, expect, it } from "vitest";
import { verifyTauriMinisign } from "./minisign-verify.mjs";
import {
  reuseGithubReleaseIdentitySignature,
  reuseReleaseIdentitySignature,
} from "./reuse-release-identity-signature.mjs";

const RAW_PUBLIC_KEY =
  "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
const RAW_SIGNATURE = `untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1633700835\tfile:test\tprehashed
wLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==`;
const TAURI_PUBLIC_KEY = Buffer.from(
  `untrusted comment: minisign public key\n${RAW_PUBLIC_KEY}\n`,
).toString("base64");
const TAURI_SIGNATURE = Buffer.from(`${RAW_SIGNATURE}\n`).toString("base64");

const response = (body, status = 200) =>
  new Response(body, {
    status,
    headers: body == null ? {} : { "content-length": String(Buffer.byteLength(body)) },
  });

describe("release identity minisign", () => {
  it("verifies the Tauri base64 minisign envelope and rejects changed bytes", () => {
    expect(verifyTauriMinisign(Buffer.from("test"), TAURI_SIGNATURE, TAURI_PUBLIC_KEY)).toBe(true);
    expect(() =>
      verifyTauriMinisign(Buffer.from("tampered"), TAURI_SIGNATURE, TAURI_PUBLIC_KEY),
    ).toThrow(/content signature verification failed/);
  });

  it("reuses the exact first-stage signature on a release rerun", async () => {
    const identityBytes = Buffer.from("test");
    const missingFetch = async () => response(null, 404);
    await expect(
      reuseReleaseIdentitySignature({
        identityBytes,
        version: "1.0.0",
        mirrorBase: "https://mirror.example/manager",
        publicKey: TAURI_PUBLIC_KEY,
        fetchImpl: missingFetch,
      }),
    ).resolves.toBeNull();

    const rerunFetch = async (url) =>
      response(url.pathname.endsWith(".sig") ? TAURI_SIGNATURE : identityBytes);
    const reused = await reuseReleaseIdentitySignature({
      identityBytes,
      version: "1.0.0",
      mirrorBase: "https://mirror.example/manager",
      publicKey: TAURI_PUBLIC_KEY,
      fetchImpl: rerunFetch,
    });

    expect(reused.toString("utf8")).toBe(`${TAURI_SIGNATURE}\n`);
  });

  it("reuses an already-published GitHub Release signature before consulting mirrors", async () => {
    const identityBytes = Buffer.from("test");
    const calls = [];
    const fetchImpl = async (url) => {
      calls.push(String(url));
      if (String(url).includes("/releases/tags/v1.0.0")) {
        return response(
          JSON.stringify({
            assets: [
              { name: "release-identity.json", url: "https://api.github.test/assets/1" },
              { name: "release-identity.json.sig", url: "https://api.github.test/assets/2" },
            ],
          }),
        );
      }
      if (String(url).endsWith("/assets/1")) return response(identityBytes);
      if (String(url).endsWith("/assets/2")) return response(TAURI_SIGNATURE);
      throw new Error(`unexpected URL ${url}`);
    };

    const reused = await reuseGithubReleaseIdentitySignature({
      identityBytes,
      version: "1.0.0",
      repository: "owner/repo",
      tag: "v1.0.0",
      token: "test-token",
      publicKey: TAURI_PUBLIC_KEY,
      fetchImpl,
    });

    expect(reused.toString("utf8")).toBe(`${TAURI_SIGNATURE}\n`);
    expect(calls).toEqual([
      "https://api.github.com/repos/owner/repo/releases/tags/v1.0.0",
      "https://api.github.test/assets/1",
      "https://api.github.test/assets/2",
    ]);
  });

  it("fails closed when an immutable prior identity differs", async () => {
    const fetchImpl = async (url) =>
      response(url.pathname.endsWith(".sig") ? TAURI_SIGNATURE : "different");
    await expect(
      reuseReleaseIdentitySignature({
        identityBytes: Buffer.from("test"),
        version: "1.0.0",
        mirrorBase: "https://mirror.example/manager",
        publicKey: TAURI_PUBLIC_KEY,
        fetchImpl,
      }),
    ).rejects.toThrow(/immutable release identity differs/);
  });
});
