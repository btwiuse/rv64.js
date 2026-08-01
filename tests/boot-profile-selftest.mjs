import assert from "node:assert/strict";
import { BootTimeline, median, summarizeTrials } from "./boot-profile-lib.mjs";

let now = 100;
let instructions = 10n;
const timeline = new BootTimeline(
  { firmware: /OpenSBI v/, kernel: /Linux version/, ready: /BENCH_READY/ },
  () => now,
);
now = 110;
timeline.write("OpenS", () => instructions);
timeline.mark("kernelEntry", () => instructions);
now = 120;
instructions = 20n;
timeline.write("BI v1.4\nLinux ver", () => instructions);
now = 140;
instructions = 40n;
timeline.write("sion 6.12\nBENCH_READY\n", () => instructions);
assert.deepEqual(timeline.milestones, {
  kernelEntry: { elapsedMs: 10, instructions: 10 },
  firmware: { elapsedMs: 20, instructions: 20 },
  kernel: { elapsedMs: 40, instructions: 40 },
  ready: { elapsedMs: 40, instructions: 40 },
});
assert.equal(median([9, 1, 5]), 5);
assert.equal(median([9, 1, 5, 3]), 4);
assert.deepEqual(
  summarizeTrials([
    { wasmCreateMs: 4, machineBuildMs: 8, milestones: { ready: { elapsedMs: 30, instructions: 300 } } },
    { wasmCreateMs: 2, machineBuildMs: 6, milestones: { ready: { elapsedMs: 20, instructions: 200 } } },
  ]),
  {
    wasmCreateMedianMs: 3,
    machineBuildMedianMs: 7,
    milestones: { ready: { reached: 2, medianMs: 25, medianInstructions: 250 } },
    phases: {},
  },
);
console.log("boot-profile self-test: PASS");
