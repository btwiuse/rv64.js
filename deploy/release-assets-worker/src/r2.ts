const IMMUTABLE_CACHE = "public, max-age=31536000, immutable";

interface ParsedRange {
  length: number;
  offset: number;
}

function parseRange(value: string, size: number): ParsedRange | null {
  const match = /^bytes=(\d*)-(\d*)$/.exec(value);
  if (!match) return null;
  const startText = match[1] ?? "";
  const endText = match[2] ?? "";
  if (!startText && !endText) return null;

  if (!startText) {
    const suffix = Number(endText);
    if (!Number.isSafeInteger(suffix) || suffix <= 0) return null;
    const length = Math.min(suffix, size);
    return { length, offset: size - length };
  }

  const offset = Number(startText);
  const requestedEnd = endText ? Number(endText) : size - 1;
  if (
    !Number.isSafeInteger(offset) ||
    !Number.isSafeInteger(requestedEnd) ||
    offset < 0 ||
    offset >= size ||
    requestedEnd < offset
  ) {
    return null;
  }
  const end = Math.min(requestedEnd, size - 1);
  return { length: end - offset + 1, offset };
}

function baseHeaders(object: R2Object): Headers {
  const headers = new Headers();
  object.writeHttpMetadata(headers);
  headers.set("Accept-Ranges", "bytes");
  headers.set("Access-Control-Allow-Origin", "*");
  headers.set("Cache-Control", IMMUTABLE_CACHE);
  headers.set("ETag", object.httpEtag);
  return headers;
}

export async function serveObject(
  bucket: R2Bucket,
  key: string,
  request: Request,
): Promise<Response | null> {
  const metadata = await bucket.head(key);
  if (!metadata) return null;

  const headers = baseHeaders(metadata);
  if (request.headers.get("If-None-Match") === metadata.httpEtag) {
    return new Response(null, { headers, status: 304 });
  }
  if (request.method === "HEAD") {
    headers.set("Content-Length", String(metadata.size));
    return new Response(null, { headers });
  }

  const rangeHeader = request.headers.get("Range");
  if (rangeHeader) {
    const range = parseRange(rangeHeader, metadata.size);
    if (!range) {
      headers.set("Content-Range", `bytes */${String(metadata.size)}`);
      return new Response(null, { headers, status: 416 });
    }
    const object = await bucket.get(key, { range });
    if (!object) return null;
    headers.set("Content-Length", String(range.length));
    headers.set(
      "Content-Range",
      `bytes ${String(range.offset)}-${String(range.offset + range.length - 1)}/${String(metadata.size)}`,
    );
    return new Response(object.body, { headers, status: 206 });
  }

  const object = await bucket.get(key);
  if (!object) return null;
  headers.set("Content-Length", String(object.size));
  return new Response(object.body, { headers });
}
