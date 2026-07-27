# HTTP fallback relay

`connectHttpRelay()` is the optional CORS fallback for the in-browser proxy.
It is separate from `connectNet()`: that socket carries raw Ethernet frames,
while this one carries already-parsed HTTP requests.

Start the included loopback relay:

```sh
node web/http-relay.mjs --port 8090
```

Then attach it to the VM:

```js
vm.connectHttpRelay("ws://127.0.0.1:8090");
vm.bootLinux({ bios, kernel, disk, proxy: true });
```

The server binds to `127.0.0.1` and accepts browser connections only from
localhost origins by default. A remotely served page requires an exact
`--allow-origin https://example.test`; an HTTPS page also needs the relay
behind a `wss://` reverse proxy.

Fetch remains the first route. A GET or HEAD that fails before exposing a
response head is safe to retry and causes its origin to be remembered for the
relay. Methods such as POST are not retried automatically because the first
request may have reached the server despite its response being hidden by CORS.
Route those origins before issuing the request:

```js
vm.routeHttpViaRelay("https://api.example.test");
```

## Wire protocol

Each WebSocket binary message starts with a 16-byte header:

| Offset | Size | Meaning |
|---:|---:|---|
| 0 | 4 | ASCII `RHR1` |
| 4 | 1 | Message type |
| 5 | 3 | Reserved, zero |
| 8 | 8 | Request ID, little-endian |
| 16 | ... | Payload |

Client type `1` is the length-prefixed `httpproxy::Request::encode` form.
Server types are `2` response head (`httpproxy::encode_head`), `3` raw body
chunk, `4` end, and `5` UTF-8 error. Different request IDs may be interleaved;
messages for one request remain ordered.
