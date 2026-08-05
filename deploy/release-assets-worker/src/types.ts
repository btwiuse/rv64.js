export interface Env {
  ASSETS: R2Bucket;
  COORDINATOR: DurableObjectNamespace;
  GITHUB_TOKEN?: string;
  ALIAS_TTL_SECONDS: string;
  EDGE_ALIAS_TTL_SECONDS: string;
  GITHUB_REPOSITORY: string;
  R2_PUBLIC_BASE_URL: string;
  R2_REDIRECT_TTL_SECONDS: string;
}

export type Alias = "latest" | "prelease";

export interface AliasState {
  checkedAt: number;
  etag?: string;
  tag: string;
}

export interface GitHubRelease {
  draft: boolean;
  prerelease: boolean;
  published_at: string | null;
  tag_name: string;
}

export interface FillRequest {
  asset: string;
  key: string;
  tag: string;
}
