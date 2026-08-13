#!/usr/bin/env node

import assert from "node:assert/strict";
import { RV64Debug } from "../web/rv64.js";

const memory = { buffer: new ArrayBuffer(4096) };
const requests = [
  Uint8Array.from([7, 0, 0, 0, 100, 3, 0]),
  Uint8Array.from([7, 0, 0, 0, 116, 9, 0]),
];
const replies = [];
let stagedLength = 0;

const ex = {
  memory,
  staging_ptr: () => 0,
  staging_alloc: (length) => {
    stagedLength = length;
    return 0;
  },
  virt_p9_take_request: () => {
    const request = requests.shift();
    if (!request) return 0;
    new Uint8Array(memory.buffer, 0, request.length).set(request);
    return request.length;
  },
  virt_p9_reply: () => {
    replies.push(new Uint8Array(memory.buffer, 0, stagedLength).slice());
    return 1;
  },
};

const vm = new RV64Debug({ exports: ex });
const completions = new Map();
const flushPromises = () => new Promise((resolve) => setImmediate(resolve));
vm.onP9Request = (request) =>
  new Promise((resolve) => completions.set(request[5], resolve));

vm.pumpP9();
assert.equal(vm.p9Pending, 2, "the bridge starts both requests without awaiting either");
await Promise.resolve();
assert.deepEqual([...completions.keys()], [3, 9], "requests retain FIFO submission order");

completions.get(9)(Uint8Array.from([7, 0, 0, 0, 117, 9, 0]));
await flushPromises();
assert.equal(replies.length, 1);
assert.equal(replies[0][5], 9, "the second tag may complete first");

completions.get(3)(Uint8Array.from([7, 0, 0, 0, 101, 3, 0]));
await flushPromises();
assert.deepEqual(
  replies.map((reply) => reply[5]),
  [9, 3],
  "host completions are forwarded in resolution order",
);
assert.equal(vm.p9Pending, 0);

console.log("PASS concurrent external 9P browser bridge");
