# Cloudflare release asset cache

This Worker exposes a configured GitHub repository's Release assets through
short, stable URLs and uses R2 as a disposable read-through cache:

```text
https://<worker>/demo-images-v2/modern-Image
https://<worker>/latest/modern-Image
https://<worker>/prelease/modern-Image
```

Concrete URLs are stored under `releases/{tag}/{asset}` in R2. The `latest` and
`prelease` virtual tags resolve from GitHub, issue a `302` to the concrete URL,
and are never used as R2 keys. `latest` uses GitHub's public latest-download
redirect and selects the latest stable release. `prelease` uses the Releases API
and selects the most recently published release marked as a prerelease. Draft
releases are not visible.

The Worker checks R2 first. A concrete cache miss is fetched from the configured
GitHub repository, stored in R2, and then served. A Durable Object provides a
global short-lived alias cache and a single-flight lock for each cold asset so
concurrent requests do not download the same object repeatedly. With the
default private-R2 delivery, successful objects support `GET`, `HEAD`, ETags,
CORS, and single byte ranges.

## Validate

Use Node.js 22 or newer:

```sh
cd deploy/release-assets-worker
npm install
npm run check
npm run dev
```

`npm run check` runs strict TypeScript typechecking, type-aware ESLint rules,
Prettier verification, and the test suite.

## Deploy

You need a Cloudflare account with R2 enabled. If this is your first R2 project,
open **Storage & databases > R2** in the Cloudflare dashboard and complete the
R2 subscription checkout. The included free tier still applies.

Install dependencies, authenticate, create the production bucket once, and
deploy:

```sh
cd deploy/release-assets-worker
npm install
npx wrangler login
npx wrangler r2 bucket create rv64-release-assets
npm run check
npm run deploy
```

Wrangler prints the deployed `workers.dev` URL. The configured Durable Object
uses SQLite storage and is available on both Workers Free and Paid plans. The
`v1` migration in `wrangler.jsonc` creates it during the first deployment.

The `rv64-release-assets-dev` preview bucket is only needed for remote
development. Normal `npm run dev` uses local R2 emulation. Create the preview
bucket if you choose to use remote bindings:

```sh
npx wrangler r2 bucket create rv64-release-assets-dev
```

`/latest` does not consume GitHub API quota. A token is recommended, and is
required for reliable `/prelease` resolution because GitHub has no equivalent
public prerelease redirect. Create a fine-grained token with public read access
to the configured repository and store it as a Worker secret after the first
deploy:

```sh
npx wrangler secret put GITHUB_TOKEN
```

That command creates and deploys a new Worker version containing the secret.
Secrets are preserved by later `npm run deploy` commands. For local development,
put `GITHUB_TOKEN=...` in an ignored `.dev.vars` file instead.

No release manifest or prewarming job is required. GitHub is the release source
of truth. `ALIAS_TTL_SECONDS` bounds global GitHub checks and
`EDGE_ALIAS_TTL_SECONDS` controls redirect caching at each Cloudflare location.
Both default to 60 seconds.

`GITHUB_REPOSITORY` is the single source-repository setting and must have the
form `owner/repository`. Change it in `wrangler.jsonc` when the repository moves;
no Worker code changes are required. Its strict validation also prevents the
Worker from becoming a configurable open proxy.

### Verify the deployment

Replace the example hostname with the URL printed by Wrangler. The first `GET`
populates R2 from GitHub; the second reads the versioned object from R2:

```sh
export RV64_ASSET_WORKER=https://rv64-release-assets.<account>.workers.dev
curl --fail --location "$RV64_ASSET_WORKER/latest/SHA256SUMS"
curl --fail --location "$RV64_ASSET_WORKER/demo-images-v2/SHA256SUMS"
curl --fail --range 0-31 "$RV64_ASSET_WORKER/demo-images-v2/SHA256SUMS"
```

`/latest/{asset}` returns a temporary redirect to `/{resolved-tag}/{asset}`.
`/prelease/{asset}` returns `404` when the repository has no published
prerelease.

### Optional direct R2 delivery

By default, the Worker streams cached objects from its private R2 binding. This
requires no domain and does not incur R2 or Worker bandwidth charges.

After attaching a production custom domain directly to the R2 bucket, set its
HTTPS origin in `R2_PUBLIC_BASE_URL`:

```json
"R2_PUBLIC_BASE_URL": "https://objects.example.com"
```

The Worker will continue resolving aliases, checking R2, and filling cold
objects from GitHub. Once an object exists, it returns a `302` directly to
`https://objects.example.com/releases/{tag}/{asset}` instead of proxying its
body. `R2_REDIRECT_TTL_SECONDS` controls the redirect's browser/CDN lifetime and
defaults to one hour. Leave `R2_PUBLIC_BASE_URL` empty to retain Worker delivery.

## R2 expiration

R2 is only a cache. An optional lifecycle rule can remove objects 90 days after
they were populated. An expired asset is downloaded from GitHub again on its
next request:

```sh
npx wrangler r2 bucket lifecycle add rv64-release-assets \
  expire-release-cache releases/ --expire-days 90
```

R2 lifecycle age is based on object creation rather than last access. A popular
object is therefore refetched once after each lifecycle expiration.
