# Cloudflare Worker HTTP relay

This deploys rv64.js's request-level CORS fallback. It is deliberately not a
general network proxy: the checked-in configuration accepts the rv64.js GitHub
Pages origin and localhost development pages, permits only HTTPS `GET`/`HEAD`,
and permits only `dl-cdn.alpinelinux.org` upstream.

```sh
cd deploy/http-relay-worker
npm install
npx wrangler login
npm run deploy
```

Validate the complete Worker locally before deployment:

```sh
npm test
```

This starts `wrangler dev`, checks the health endpoint, verifies rejected and
accepted browser origins, and performs a real request to the Alpine CDN through
the binary relay protocol.

Wrangler prints a URL such as `https://rv64-http-relay.example.workers.dev`.
Use its WebSocket form with the `/relay` path:

```js
network: {
  mode: "fetch",
  relayURL: "wss://rv64-http-relay.example.workers.dev/relay",
}
```

For the bundled demo, paste that URL into `web/site-config.js`. A `?relay=wss://…`
query parameter still overrides the configured value for development.

Before deploying from a fork, change `ALLOWED_ORIGINS` in `wrangler.jsonc` to
your Pages origin. Origins never include a path, so
`https://ibuildthecloud.github.io/rv64.js/` uses
`https://ibuildthecloud.github.io`.

`ALLOWED_HOSTS` is a comma-separated exact-hostname allowlist. Redirects are
validated against it at every hop. Add a mirror hostname only after observing
an actual redirect failure; avoid broad suffix or wildcard rules.

The relay does not own or receive the guest proxy CA. rv64.js creates a fresh
CA inside each VM instance, exposes only its public certificate to that guest,
and sends the relay an already-decoded HTTP request.
