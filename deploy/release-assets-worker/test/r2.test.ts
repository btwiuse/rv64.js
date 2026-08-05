import { describe, expect, it, vi } from "vitest";

import { serveObject } from "../src/r2";

const CONTENT = new TextEncoder().encode("0123456789");

function metadata(): R2Object {
  return {
    checksums: {} as R2Checksums,
    customMetadata: {},
    etag: "abc",
    httpEtag: '"abc"',
    httpMetadata: { contentType: "application/octet-stream" },
    key: "releases/v1/disk.img",
    size: CONTENT.length,
    storageClass: "Standard",
    uploaded: new Date(0),
    version: "version",
    writeHttpMetadata(headers): void {
      headers.set("Content-Type", "application/octet-stream");
    },
  };
}

function bucket(): R2Bucket {
  const object = metadata();
  return {
    get: vi.fn(async (_key: string, options?: R2GetOptions) => {
      const range = options?.range as R2Range | undefined;
      const offset = range && "offset" in range ? range.offset : 0;
      const length = range && "length" in range ? range.length : CONTENT.length;
      const body = new Blob([CONTENT.slice(offset, offset + length)]).stream();
      return { ...object, body, bodyUsed: false, range } as R2ObjectBody;
    }),
    head: vi.fn(async () => object),
  } as unknown as R2Bucket;
}

describe("R2 responses", () => {
  it("serves a complete immutable object", async () => {
    const response = await serveObject(
      bucket(),
      "releases/v1/disk.img",
      new Request("https://x/v1/disk.img"),
    );
    expect(response?.status).toBe(200);
    expect(await response?.text()).toBe("0123456789");
    expect(response?.headers.get("Cache-Control")).toContain("immutable");
    expect(response?.headers.get("Access-Control-Allow-Origin")).toBe("*");
  });

  it("serves byte and suffix ranges", async () => {
    const first = await serveObject(
      bucket(),
      "releases/v1/disk.img",
      new Request("https://x/v1/disk.img", { headers: { Range: "bytes=2-5" } }),
    );
    expect(first?.status).toBe(206);
    expect(first?.headers.get("Content-Range")).toBe("bytes 2-5/10");
    expect(await first?.text()).toBe("2345");

    const suffix = await serveObject(
      bucket(),
      "releases/v1/disk.img",
      new Request("https://x/v1/disk.img", { headers: { Range: "bytes=-3" } }),
    );
    expect(await suffix?.text()).toBe("789");
  });

  it("rejects invalid ranges and honors conditional requests", async () => {
    const invalid = await serveObject(
      bucket(),
      "releases/v1/disk.img",
      new Request("https://x/v1/disk.img", {
        headers: { Range: "bytes=20-30" },
      }),
    );
    expect(invalid?.status).toBe(416);
    expect(invalid?.headers.get("Content-Range")).toBe("bytes */10");

    const unchanged = await serveObject(
      bucket(),
      "releases/v1/disk.img",
      new Request("https://x/v1/disk.img", {
        headers: { "If-None-Match": '"abc"' },
      }),
    );
    expect(unchanged?.status).toBe(304);
  });
});
