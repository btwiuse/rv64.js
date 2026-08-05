import { describe, expect, it } from "vitest";

import { publicObjectRedirect, publicObjectUrl } from "../src/public-r2";
import type { Env } from "../src/types";

function env(baseUrl: string): Env {
  return {
    R2_PUBLIC_BASE_URL: baseUrl,
    R2_REDIRECT_TTL_SECONDS: "1800",
  } as Env;
}

describe("optional public R2 redirects", () => {
  it("is disabled when no public origin is configured", () => {
    expect(publicObjectUrl(env(""), "releases/v1/disk.img")).toBeNull();
    expect(publicObjectRedirect(env(""), "releases/v1/disk.img")).toBeNull();
  });

  it("builds a cacheable URL from the versioned R2 key", () => {
    const configured = env("https://objects.example.com");
    expect(
      publicObjectUrl(configured, "releases/demo images/v2/disk image.img"),
    ).toBe(
      "https://objects.example.com/releases/demo%20images/v2/disk%20image.img",
    );
    const response = publicObjectRedirect(configured, "releases/v2/disk.img");
    expect(response?.status).toBe(302);
    expect(response?.headers.get("Location")).toBe(
      "https://objects.example.com/releases/v2/disk.img",
    );
    expect(response?.headers.get("Cache-Control")).toBe("public, max-age=1800");
    expect(response?.headers.get("Access-Control-Allow-Origin")).toBe("*");
  });

  it("rejects unsafe or ambiguous public base URLs", () => {
    expect(() =>
      publicObjectUrl(env("http://objects.example.com"), "releases/v1/a"),
    ).toThrow();
    expect(() =>
      publicObjectUrl(
        env("https://objects.example.com/prefix"),
        "releases/v1/a",
      ),
    ).toThrow();
    expect(() =>
      publicObjectUrl(env("https://user@example.com"), "releases/v1/a"),
    ).toThrow();
  });
});
