import assert from "node:assert/strict";

import { resolveAssetOverride } from "../web/asset-source.mjs";

const production = "https://ibuildthecloud.github.io/rv64.js/";

assert.equal(resolveAssetOverride(null, production), null);
assert.equal(resolveAssetOverride("https://attacker.example/assets", production), null);
assert.equal(
  resolveAssetOverride("/rv64.js/test-assets/", production),
  "https://ibuildthecloud.github.io/rv64.js/test-assets",
);
assert.equal(
  resolveAssetOverride("https://assets.example/releases", "http://localhost:8000/web/"),
  "https://assets.example/releases",
);
assert.equal(
  resolveAssetOverride("http://127.0.0.1:9000/assets/", "http://[::1]:8000/web/"),
  "http://127.0.0.1:9000/assets",
);
assert.equal(resolveAssetOverride("file:///tmp/assets", "http://localhost:8000/web/"), null);
assert.equal(resolveAssetOverride("https://user:pass@example.com", "http://localhost:8000/"), null);
assert.equal(resolveAssetOverride("https://assets.example/path?q=1", "http://localhost:8000/"), null);

console.log("PASS demo asset source policy");
