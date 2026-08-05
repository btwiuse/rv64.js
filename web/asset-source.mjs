const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

export function resolveAssetOverride(rawOverride, pageLocation) {
  if (!rawOverride) return null;

  let page;
  let candidate;
  try {
    page = new URL(pageLocation);
    candidate = new URL(rawOverride, page);
  } catch {
    return null;
  }

  if (candidate.protocol !== "http:" && candidate.protocol !== "https:") return null;
  if (candidate.username || candidate.password || candidate.search || candidate.hash) return null;
  if (!LOCAL_HOSTS.has(page.hostname) && candidate.origin !== page.origin) return null;
  return candidate.href.replace(/\/$/, "");
}
