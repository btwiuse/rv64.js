#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const source = await readFile(
  new URL("../integrations/wanix/adapter/p9-handler.js", import.meta.url),
  "utf8",
);
const createP9Handler = (0, eval)(source);

const packet = (type, tag) => Uint8Array.from([
  7, 0, 0, 0, type, tag & 0xff, tag >>> 8,
]);
const port = {
  sent: [],
  postMessage(request) {
    if (this.throwTag === (request[5] | (request[6] << 8))) {
      this.throwTag = null;
      throw new Error("synthetic post failure");
    }
    this.sent.push(request.slice());
  },
};
const handle = createP9Handler(port);

const first = handle(packet(116, 1));
const second = handle(packet(118, 2));
assert.deepEqual(port.sent.map((request) => request[5]), [1],
  "only the queue head reaches the WANIX stream endpoint");

port.onmessage({ data: packet(117, 99) });
assert.deepEqual(port.sent.map((request) => request[5]), [1],
  "an unrelated response cannot release the queue head");

port.onmessage({ data: packet(117, 1) });
assert.deepEqual(await first, packet(117, 1));
assert.deepEqual(port.sent.map((request) => request[5]), [1, 2],
  "the next request starts only after the matching response");

await assert.rejects(handle(packet(100, 2)), /9P tag collision: 2/);
port.onmessage({ data: packet(119, 2) });
assert.deepEqual(await second, packet(119, 2));

port.throwTag = 3;
const failed = handle(packet(116, 3));
const afterFailure = handle(packet(116, 4));
await assert.rejects(failed, /synthetic post failure/);
await new Promise((resolve) => setImmediate(resolve));
assert.deepEqual(port.sent.map((request) => request[5]), [1, 2, 4],
  "a synchronous post failure does not wedge the queue");
port.onmessage({ data: packet(117, 4) });
assert.deepEqual(await afterFailure, packet(117, 4));

console.log("PASS WANIX external 9P endpoint is FIFO single-flight");
