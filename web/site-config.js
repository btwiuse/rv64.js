// Paste the deployed Cloudflare Worker WebSocket URL here. The query parameter
// ?relay=wss://... overrides it, and an empty string retains direct fetch only.
export const defaultRelayURL =
  "wss://rv64-http-relay.darren-e4d.workers.dev/relay";

// Release assets are served from the R2-backed cache Worker. On localhost,
// ?assets=https://... may select another development origin; deployed pages
// accept only same-origin overrides.
export const defaultAssetURL =
  "https://rv64-release-assets.darren-e4d.workers.dev/latest";
