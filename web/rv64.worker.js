import { RV64 } from "./rv64.js";

const EVENTS = [
  "ready",
  "start",
  "stop",
  "error",
  "console",
  "networkTransmit",
  "networkTraffic",
  "downloadProgress",
];

let vm = null;
let statisticsTimer = null;

function serializeError(error) {
  return {
    message: String(error?.message ?? error),
    name: error?.name ?? "Error",
    stack: error?.stack,
  };
}

function state() {
  return {
    instructions: vm ? vm.instructions : 0n,
    proxyURL: vm?.network.proxyURL,
    running: vm?.running ?? false,
  };
}

function postEvent(event, detail) {
  self.postMessage({
    type: "event",
    event,
    detail: event === "error" ? serializeError(detail) : detail,
  });
}

async function create(message) {
  const events = Object.fromEntries(
    EVENTS.map((event) => [event, (detail) => postEvent(event, detail)]),
  );
  vm = await RV64.create({
    ...message.options,
    events,
    execution: { mode: "local" },
  });
  const interval = Number(message.statisticsIntervalMs);
  statisticsTimer = setInterval(() => {
    if (vm) self.postMessage({ type: "state", state: state() });
  }, Number.isFinite(interval) && interval >= 50 ? interval : 500);
  self.postMessage({ type: "created", state: state() });
}

async function call(message) {
  try {
    let value;
    if (message.method === "start") value = await vm.start();
    else if (message.method === "stop") value = await vm.stop();
    else if (message.method === "reset") value = await vm.reset();
    else if (message.method === "setJitEnabled") {
      value = await vm.setJitEnabled(message.value);
    }
    else if (message.method === "jitStats") value = await vm.jitStats();
    else if (message.method === "jitProfile") value = await vm.jitProfile(message.value);
    else if (message.method === "configureJit") value = await vm.configureJit(message.value);
    else if (message.method === "destroy") {
      value = await vm.destroy();
      vm = null;
      clearInterval(statisticsTimer);
      statisticsTimer = null;
    } else throw new Error(`unknown Worker method: ${message.method}`);
    self.postMessage({ id: message.id, type: "result", value, state: state() });
  } catch (error) {
    self.postMessage({
      id: message.id,
      type: "result",
      error: serializeError(error),
      state: state(),
    });
  }
}

self.onmessage = (event) => {
  const message = event.data;
  if (message?.type === "create") {
    create(message).catch((error) => {
      self.postMessage({ type: "create-error", error: serializeError(error) });
    });
  } else if (message?.type === "call") {
    call(message);
  } else if (message?.type === "console") {
    vm?.console.send(message.value);
  } else if (message?.type === "network-receive") {
    vm?.network.receive(message.value);
  }
};
