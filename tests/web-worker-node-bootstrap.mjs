import { parentPort, workerData } from "node:worker_threads";

const queued = [];
globalThis.self = globalThis;
globalThis.postMessage = (message, transfers) =>
  parentPort.postMessage(message, transfers);
parentPort.on("message", (data) => {
  if (typeof globalThis.onmessage === "function") globalThis.onmessage({ data });
  else queued.push(data);
});

await import(workerData.target);
for (const data of queued.splice(0)) globalThis.onmessage({ data });
