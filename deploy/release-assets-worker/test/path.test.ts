import { describe, expect, it } from "vitest";

import { asAlias, objectKey, parseAssetPath, versionedPath } from "../src/path";

describe("release asset paths", () => {
  it("parses a concrete release tag and asset", () => {
    expect(parseAssetPath("/demo-images-v2/modern-Image")).toEqual({
      asset: "modern-Image",
      tag: "demo-images-v2",
    });
  });

  it("rejects paths that could escape the two-segment namespace", () => {
    expect(parseAssetPath("/tag/directory/asset")).toBeNull();
    expect(parseAssetPath("/../asset")).toBeNull();
    expect(parseAssetPath("/tag/%2Fetc")).toBeNull();
    expect(parseAssetPath("/tag/asset%00")).toBeNull();
  });

  it("recognizes only the two virtual tags", () => {
    expect(asAlias("latest")).toBe("latest");
    expect(asAlias("prelease")).toBe("prelease");
    expect(asAlias("demo-images-v2")).toBeNull();
  });

  it("always versions R2 keys by the resolved release tag", () => {
    expect(objectKey("demo-images-v2", "modern-Image")).toBe(
      "releases/demo-images-v2/modern-Image",
    );
    expect(versionedPath("demo-images-v2", "modern-Image")).toBe(
      "/demo-images-v2/modern-Image",
    );
  });
});
