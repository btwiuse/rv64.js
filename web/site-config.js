// Paste the deployed Cloudflare Worker WebSocket URL here. The query parameter
// ?relay=wss://... overrides it, and an empty string retains direct fetch only.
export const defaultRelayURL =
  "wss://rv64-http-relay.darren-e4d.workers.dev/relay";

// Demo images are pinned to their immutable image release. The Worker's
// generic /latest alias also sees semver library releases, which intentionally
// do not contain the kernel or disk. On localhost, ?assets=https://... may
// select another development origin; deployed pages accept only same-origin
// overrides.
export const defaultAssetURL =
  "https://rv64-release-assets.darren-e4d.workers.dev/demo-images-v4";
