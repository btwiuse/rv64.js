import type { Env } from "./types";

function positiveInteger(value: string, fallback: number): number {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : fallback;
}

export function publicObjectUrl(env: Env, key: string): string | null {
  const configured = env.R2_PUBLIC_BASE_URL.trim();
  if (!configured) return null;

  const base = new URL(configured);
  if (
    base.protocol !== "https:" ||
    base.username ||
    base.password ||
    base.search ||
    base.hash ||
    (base.pathname !== "/" && base.pathname !== "")
  ) {
    throw new Error(
      "R2_PUBLIC_BASE_URL must be an HTTPS origin without a path",
    );
  }
  return `${base.origin}/${key.split("/").map(encodeURIComponent).join("/")}`;
}

export function publicObjectRedirect(env: Env, key: string): Response | null {
  const location = publicObjectUrl(env, key);
  if (!location) return null;
  const ttl = positiveInteger(env.R2_REDIRECT_TTL_SECONDS, 3600);
  return new Response(null, {
    headers: {
      "Access-Control-Allow-Origin": "*",
      "Cache-Control": `public, max-age=${String(ttl)}`,
      Location: location,
    },
    status: 302,
  });
}
