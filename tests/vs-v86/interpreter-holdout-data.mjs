#!/usr/bin/env node

// Generate the immutable data and result contract for the interpreter
// holdouts. This script is deterministic and does not execute either emulator.

import { createCipheriv, createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const output = process.argv[2] && resolve(process.argv[2]);
if (!output) throw new Error("usage: interpreter-holdout-data.mjs OUTPUT_DIR");
await mkdir(output, { recursive: true });

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const corpus = Buffer.alloc(1024 * 1024);
let state = 0x6d2b79f5;
for (let index = 0; index < corpus.length; index++) {
  state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
  const lane = index & 127;
  corpus[index] = lane < 80
    ? (index * 13 + (index >>> 9) * 29) & 0xff
    : state >>> 24;
}

const records = [];
state = 0x243f6a88;
for (let index = 0; index < 75_000; index++) {
  state = (Math.imul(state, 1103515245) + 12345) >>> 0;
  const key = state.toString(16).padStart(8, "0");
  state = (Math.imul(state, 1103515245) + 12345) >>> 0;
  const payload = state.toString(16).padStart(8, "0");
  records.push(`${key}\t${String(74_999 - index).padStart(5, "0")}\t${payload}\n`);
}
const recordBytes = Buffer.from(records.join(""));
const sortedBytes = Buffer.from(records.toSorted().join(""));

const aesKey = Buffer.from(
  "00112233445566778899aabbccddeeff" +
  "102132435465768798a9bacbdcedfe0f",
  "hex",
);
const aesIv = Buffer.from("0f0e0d0c0b0a09080706050403020100", "hex");
const cipher = createCipheriv("aes-256-ctr", aesKey, aesIv);
const encrypted = Buffer.concat([cipher.update(corpus), cipher.final()]);

const contract = {
  schema: 1,
  population: "sealed-interpreter-holdouts-v1",
  tuningUse: "forbidden",
  evaluationRule: "run only after the candidate is frozen",
  data: {
    corpus: { bytes: corpus.length, sha256: sha256(corpus) },
    records: { records: records.length, bytes: recordBytes.length, sha256: sha256(recordBytes) },
  },
  expected: {
    holdout_gzip: sha256(corpus),
    holdout_sort: sha256(sortedBytes),
    holdout_sha256: sha256(corpus),
    holdout_aes: sha256(encrypted),
  },
  work: {
    holdout_gzip: { inputBytes: corpus.length, repetitions: 4 },
    holdout_sort: { records: records.length, repetitions: 1, locale: "C" },
    holdout_sha256: { inputBytes: corpus.length, repetitions: 32 },
    holdout_aes: { inputBytes: corpus.length, repetitions: 16, cipher: "AES-256-CTR" },
  },
};

const env = [
  `CORPUS_SHA256=${contract.data.corpus.sha256}`,
  `SORTED_SHA256=${contract.expected.holdout_sort}`,
  `AES_SHA256=${contract.expected.holdout_aes}`,
  "",
].join("\n");

await Promise.all([
  writeFile(resolve(output, "corpus.bin"), corpus),
  writeFile(resolve(output, "records.txt"), recordBytes),
  writeFile(resolve(output, "expected.env"), env),
  writeFile(resolve(output, "contract.json"), `${JSON.stringify(contract, null, 2)}\n`),
]);

process.stdout.write(`${JSON.stringify(contract, null, 2)}\n`);
