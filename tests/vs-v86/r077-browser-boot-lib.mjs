export const BOOT_QUANTUM = 2_000_000n;
export const BOOT_TIMEOUT_MS = 180_000;

export function yieldsAfterPump(pumpIndex) {
  return (pumpIndex & 3) === 0;
}

// Match scorecard-v2-worker.mjs waitUntil exactly: start after every setup
// operation, pump 2M instructions, yield after pump 1 and every fourth pump
// thereafter, and stop on the first complete ready marker.
export async function runTimedBoot({
  vm,
  ready,
  nextTask,
  now = () => performance.now(),
  timeoutMs = BOOT_TIMEOUT_MS,
}) {
  const started = now();
  const deadline = started + timeoutMs;
  let pumps = 0;
  let yields = 0;
  while (!ready()) {
    if (vm.runVirtSystem(BOOT_QUANTUM)) {
      throw new Error("guest powered off before SCORECARD_V2_READY");
    }
    if (yieldsAfterPump(pumps)) {
      yields++;
      await nextTask();
    }
    pumps++;
    if (now() > deadline) {
      throw new Error(`modern Boot exceeded ${timeoutMs} ms`);
    }
  }
  return {
    ms: now() - started,
    pumps,
    yields,
    quantum: BOOT_QUANTUM.toString(),
    cadence: "yield-after-pump-1-then-every-fourth",
    marker: "SCORECARD_V2_READY",
  };
}

