// Shared marker tracking and summary helpers for boot-profile.mjs.

export class BootTimeline {
  constructor(markers, now = () => performance.now()) {
    this.markers = markers;
    this.now = now;
    this.started = now();
    this.output = "";
    this.milestones = {};
  }

  write(text, instructionCount) {
    this.output += text;
    const elapsedMs = this.now() - this.started;
    for (const [name, pattern] of Object.entries(this.markers)) {
      if (this.milestones[name] || !pattern.test(this.output)) continue;
      this.milestones[name] = {
        elapsedMs,
        instructions: Number(instructionCount()),
      };
    }
  }

  mark(name, instructionCount) {
    if (this.milestones[name]) return;
    this.milestones[name] = {
      elapsedMs: this.now() - this.started,
      instructions: Number(instructionCount()),
    };
  }

  reached(name) {
    return Object.hasOwn(this.milestones, name);
  }
}

export function median(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length & 1
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

export function summarizeTrials(trials) {
  const milestoneNames = new Set(
    trials.flatMap((trial) => Object.keys(trial.milestones)),
  );
  const milestones = {};
  for (const name of milestoneNames) {
    const reached = trials
      .map((trial) => trial.milestones[name])
      .filter(Boolean);
    milestones[name] = {
      reached: reached.length,
      medianMs: median(reached.map((item) => item.elapsedMs)),
      medianInstructions: median(reached.map((item) => item.instructions)),
    };
  }
  const phaseDefinitions = {
    firmwareToKernelEntry: ["firmware", "kernelEntry"],
    kernelEntryToConsole: ["kernelEntry", "kernel"],
    kernelToRootMounted: ["kernel", "rootMounted"],
    rootMountedToReady: ["rootMounted", "ready"],
  };
  const phases = {};
  for (const [name, [start, end]] of Object.entries(phaseDefinitions)) {
    const values = trials.flatMap((trial) => {
      const a = trial.milestones[start];
      const b = trial.milestones[end];
      return a && b
        ? [{ elapsedMs: b.elapsedMs - a.elapsedMs, instructions: b.instructions - a.instructions }]
        : [];
    });
    if (values.length) {
      phases[name] = {
        measured: values.length,
        medianMs: median(values.map((item) => item.elapsedMs)),
        medianInstructions: median(values.map((item) => item.instructions)),
      };
    }
  }
  return {
    wasmCreateMedianMs: median(trials.map((trial) => trial.wasmCreateMs)),
    machineBuildMedianMs: median(trials.map((trial) => trial.machineBuildMs)),
    milestones,
    phases,
  };
}
