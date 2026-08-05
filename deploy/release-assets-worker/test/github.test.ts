import { afterEach, describe, expect, it, vi } from "vitest";

import { downloadUrl, fetchAlias } from "../src/github";
import type { Env } from "../src/types";

const env = {
  ALIAS_TTL_SECONDS: "60",
  GITHUB_REPOSITORY: "example/moved-rv64",
} as Env;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("GitHub release resolution", () => {
  it("constructs a repository-pinned download URL", () => {
    expect(downloadUrl(env, "demo-images-v2", "modern Image")).toBe(
      "https://github.com/example/moved-rv64/releases/download/demo-images-v2/modern%20Image",
    );
  });

  it("rejects an invalid configured repository", () => {
    expect(() =>
      downloadUrl(
        { ...env, GITHUB_REPOSITORY: "https://example.com/open-proxy" },
        "v1",
        "asset",
      ),
    ).toThrow("owner/repository");
  });

  it("selects the newest published prerelease", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json(
          [
            {
              draft: false,
              prerelease: true,
              published_at: "2026-01-01T00:00:00Z",
              tag_name: "old",
            },
            {
              draft: false,
              prerelease: true,
              published_at: "2026-02-01T00:00:00Z",
              tag_name: "new",
            },
            {
              draft: false,
              prerelease: false,
              published_at: "2026-03-01T00:00:00Z",
              tag_name: "stable",
            },
          ],
          { headers: { ETag: '"github-etag"' } },
        ),
      ),
    );
    await expect(fetchAlias("prelease", env)).resolves.toEqual({
      etag: '"github-etag"',
      notModified: false,
      tag: "new",
    });
    expect(vi.mocked(fetch).mock.calls[0]?.[0]).toBe(
      "https://api.github.com/repos/example/moved-rv64/releases?per_page=30",
    );
  });

  it("supports GitHub conditional responses", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        Promise.resolve(new Response(null, { status: 304 })),
    );
    vi.stubGlobal("fetch", fetchMock);
    await expect(fetchAlias("latest", env, '"old-etag"')).resolves.toEqual({
      notModified: true,
    });
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("If-None-Match")).toBe('"old-etag"');
  });

  it("falls back to the public latest-download redirect when the API is rate limited", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response("rate limited", { status: 403 }))
      .mockResolvedValueOnce(
        new Response(null, {
          headers: {
            Location:
              "https://github.com/example/moved-rv64/releases/download/demo-images-v3/SHA256SUMS",
          },
          status: 302,
        }),
      );
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      fetchAlias("latest", env, undefined, "SHA256SUMS"),
    ).resolves.toEqual({
      notModified: false,
      tag: "demo-images-v3",
    });
    expect(fetchMock.mock.calls[1]?.[0]).toBe(
      "https://github.com/example/moved-rv64/releases/latest/download/SHA256SUMS",
    );
    expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({
      method: "HEAD",
      redirect: "manual",
    });
  });
});
