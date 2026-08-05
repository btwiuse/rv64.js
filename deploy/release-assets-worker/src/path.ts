import type { Alias } from "./types";

const SAFE_PART = /^[A-Za-z0-9][A-Za-z0-9._+@-]{0,254}$/;

export interface AssetPath {
  asset: string;
  tag: string;
}

export function parseAssetPath(pathname: string): AssetPath | null {
  const encoded = pathname.split("/");
  if (encoded.length !== 3 || encoded[0] !== "") return null;

  try {
    const tag = decodeURIComponent(encoded[1] ?? "");
    const asset = decodeURIComponent(encoded[2] ?? "");
    if (!SAFE_PART.test(tag) || !SAFE_PART.test(asset)) return null;
    if (tag === "." || tag === ".." || asset === "." || asset === "..")
      return null;
    return { asset, tag };
  } catch {
    return null;
  }
}

export function asAlias(tag: string): Alias | null {
  if (tag === "latest" || tag === "prelease") return tag;
  return null;
}

export function objectKey(tag: string, asset: string): string {
  return `releases/${tag}/${asset}`;
}

export function versionedPath(tag: string, asset: string): string {
  return `/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`;
}
