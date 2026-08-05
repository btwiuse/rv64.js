import { Coordinator } from "./coordinator";
import { downloadUrl, fetchLatestFromDownloadRedirect } from "./github";
import { asAlias, objectKey, parseAssetPath, versionedPath } from "./path";
import { publicObjectRedirect, publicObjectUrl } from "./public-r2";
import { serveObject } from "./r2";
import type { Alias, Env, FillRequest } from "./types";

export { Coordinator };

function positiveInteger(value: string, fallback: number): number {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : fallback;
}

async function resolveAlias(
  alias: Alias,
  asset: string,
  env: Env,
): Promise<string | null> {
  if (alias === "latest") {
    const result = await fetchLatestFromDownloadRedirect(env, asset);
    return result.tag ?? null;
  }
  const id = env.COORDINATOR.idFromName(`alias:${alias}`);
  const response = await env.COORDINATOR.get(id).fetch(
    `https://coordinator/resolve/${alias}?asset=${encodeURIComponent(asset)}`,
  );
  if (response.status === 404) return null;
  if (!response.ok)
    throw new Error(`Alias coordinator returned ${String(response.status)}`);
  const result = await response.json<{ tag: string }>();
  return result.tag;
}

async function aliasRedirect(
  request: Request,
  alias: Alias,
  asset: string,
  env: Env,
): Promise<Response> {
  const cache = await caches.open("release-aliases-v2");
  const cacheKey = new Request(request.url, { method: "GET" });
  const cached = await cache.match(cacheKey);
  if (cached?.headers.has("Access-Control-Allow-Origin")) {
    const headers = new Headers(cached.headers);
    headers.set("Cache-Control", "no-store");
    return new Response(null, { headers, status: cached.status });
  }
  if (cached) await cache.delete(cacheKey);

  const tag = await resolveAlias(alias, asset, env);
  if (!tag) return new Response("No matching release", { status: 404 });
  const location = versionedPath(tag, asset);
  const edgeTtl = positiveInteger(env.EDGE_ALIAS_TTL_SECONDS, 60);
  const cacheHeaders = new Headers({
    "Access-Control-Allow-Origin": "*",
    "Cache-Control": `public, max-age=${String(edgeTtl)}`,
    Location: location,
  });
  await cache.put(
    cacheKey,
    new Response(null, { headers: cacheHeaders, status: 302 }),
  );
  return new Response(null, {
    headers: {
      "Access-Control-Allow-Origin": "*",
      "Cache-Control": "no-store",
      Location: location,
    },
    status: 302,
  });
}

async function fillAsset(input: FillRequest, env: Env): Promise<Response> {
  const id = env.COORDINATOR.idFromName(`asset:${input.key}`);
  return env.COORDINATOR.get(id).fetch("https://coordinator/fill", {
    body: JSON.stringify(input),
    headers: { "Content-Type": "application/json" },
    method: "POST",
  });
}

export async function handle(request: Request, env: Env): Promise<Response> {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("Method not allowed", {
      headers: { Allow: "GET, HEAD" },
      status: 405,
    });
  }

  const parsed = parseAssetPath(new URL(request.url).pathname);
  if (!parsed) return new Response("Expected /{tag}/{asset}", { status: 404 });
  const alias = asAlias(parsed.tag);
  if (alias) return aliasRedirect(request, alias, parsed.asset, env);

  const key = objectKey(parsed.tag, parsed.asset);
  const publicUrl = publicObjectUrl(env, key);
  if (publicUrl) {
    if (await env.ASSETS.head(key)) {
      return (
        publicObjectRedirect(env, key) ??
        new Response("Invalid redirect configuration", { status: 500 })
      );
    }
  } else {
    const cached = await serveObject(env.ASSETS, key, request);
    if (cached) return cached;
  }

  if (request.method === "HEAD") {
    const response = await fetch(downloadUrl(env, parsed.tag, parsed.asset), {
      method: "HEAD",
      redirect: "follow",
    });
    if (response.status === 404) return new Response(null, { status: 404 });
    if (!response.ok) return new Response(null, { status: 502 });
    const headers = new Headers();
    for (const name of [
      "Content-Length",
      "Content-Type",
      "ETag",
      "Last-Modified",
    ]) {
      const value = response.headers.get(name);
      if (value) headers.set(name, value);
    }
    headers.set("Access-Control-Allow-Origin", "*");
    return new Response(null, { headers });
  }

  const filled = await fillAsset({ ...parsed, key }, env);
  if (!filled.ok)
    return new Response(await filled.text(), { status: filled.status });
  const redirect = publicObjectRedirect(env, key);
  if (redirect) return redirect;
  return (
    (await serveObject(env.ASSETS, key, request)) ??
    new Response("Cache fill failed", { status: 502 })
  );
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return await handle(request, env);
    } catch (error) {
      console.error("Unhandled request failure", error);
      return new Response("Internal error", { status: 500 });
    }
  },
} satisfies ExportedHandler<Env>;
