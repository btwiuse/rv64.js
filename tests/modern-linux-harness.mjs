import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const tick = () => new Promise((done) => setImmediate(done));

export function modernImagePaths(root) {
  return {
    kernel: resolve(
      root,
      process.env.RV64_MODERN_KERNEL || "web/images/alpine/Image",
    ),
    disk: resolve(
      root,
      process.env.RV64_ALPINE_DISK || "web/images/alpine/alpine.ext4",
    ),
    opensbi: resolve(
      root,
      process.env.RV64_OPENSBI || "web/images/alpine/opensbi.bin",
    ),
  };
}

export function missingModernImages(root, mode = "opensbi") {
  const paths = modernImagePaths(root);
  const required = [paths.kernel, paths.disk];
  if (mode === "opensbi") required.push(paths.opensbi);
  return required.filter((path) => !existsSync(path));
}

export async function loadModernImages(root) {
  const paths = modernImagePaths(root);
  const [kernel, disk, opensbi] = await Promise.all([
    readFile(paths.kernel),
    readFile(paths.disk),
    readFile(paths.opensbi),
  ]);
  return {
    kernel: new Uint8Array(kernel),
    disk: new Uint8Array(disk),
    opensbi: new Uint8Array(opensbi),
  };
}

export async function bootModern({
  RV64,
  wasm,
  images,
  mode = "direct",
  jit = true,
  superblock = false,
  onJitModule,
  createOptions,
  configure,
  afterBoot,
}) {
  if (mode !== "direct" && mode !== "opensbi") {
    throw new Error(`unknown modern boot mode: ${mode}`);
  }
  const vm = await RV64.create(wasm, createOptions);
  if (onJitModule) vm.onJitModule = onJitModule;
  vm.ex.jit_set_enabled(jit ? 1 : 0);
  vm.ex.sys_set_superblock(jit && superblock ? 1 : 0);
  configure?.(vm);
  const state = { output: "", poweredOff: false };
  const decoder = new TextDecoder();
  vm.onWrite = (_fd, bytes) => {
    state.output += decoder.decode(bytes, { stream: true });
  };
  const options = {
    kernel: images.kernel,
    disk: images.disk.slice(),
    cmdline: "console=ttyS0 root=/dev/vda rw init=/rv64-init",
    ramMB: 512,
  };
  if (mode === "opensbi") {
    vm.bootVirtLinux({ ...options, opensbi: images.opensbi });
  } else {
    vm.bootVirtLinuxDirect(options);
  }
  afterBoot?.(vm);
  return {
    vm,
    state,
    mode,
    jit,
    superblock,
  };
}

export function output(machine) {
  return machine.state.output;
}

export function clearOutput(machine) {
  machine.state.output = "";
}

export async function pumpUntil(
  machine,
  done,
  { slice = 2_000_000n, timeoutMs = 180_000 } = {},
) {
  const started = performance.now();
  for (let iteration = 0; !done(); iteration++) {
    if (machine.vm.runVirtSystem(slice)) {
      machine.state.poweredOff = true;
      return done();
    }
    // Generated page functions resolve on the microtask queue. Yield every
    // slice while one is pending; otherwise a tight synchronous harness can
    // make a finished background compile wait another 32M guest instructions
    // before publication, measuring the harness rather than the JIT policy.
    if ((iteration & 15) === 0 || machine.vm.ex.sys_pending_builds?.() > 0) {
      await tick();
    }
    if (performance.now() - started > timeoutMs) return false;
  }
  return true;
}

export async function waitForAlpine(machine, timeoutMs = 180_000) {
  return pumpUntil(machine, () => output(machine).includes("ALPINE_READY"), {
    slice: 2_000_000n,
    timeoutMs,
  });
}

export async function guestCommand(
  machine,
  command,
  expected,
  timeoutMs = 60_000,
) {
  if (command.includes(expected)) {
    throw new Error(`marker ${expected} would match terminal echo`);
  }
  const start = output(machine).length;
  machine.vm.virtConsoleInput(new TextEncoder().encode(`${command}\n`));
  return pumpUntil(
    machine,
    () => output(machine).slice(start).includes(expected),
    { slice: 2_000_000n, timeoutMs },
  );
}

export async function transferBinary(machine, bytes, destination, prefix) {
  const encoded = Buffer.from(bytes).toString("base64");
  if (!await guestCommand(
    machine,
    `: > /tmp/${prefix.toLowerCase()}.b64; echo ${prefix}_'EMPTY'`,
    `${prefix}_EMPTY`,
  )) return false;
  for (let offset = 0; offset < encoded.length; offset += 512) {
    const index = offset / 512;
    const marker = `${prefix}_CHUNK_${index}`;
    const command =
      `printf %s '${encoded.slice(offset, offset + 512)}' >> /tmp/${prefix.toLowerCase()}.b64; ` +
      `echo ${prefix}_CHUNK_'${index}'`;
    if (!await guestCommand(machine, command, marker)) return false;
  }
  return guestCommand(
    machine,
    `base64 -d /tmp/${prefix.toLowerCase()}.b64 > ${destination} && ` +
      `chmod 755 ${destination}; echo ${prefix}_'DECODED'`,
    `${prefix}_DECODED`,
  );
}

export function machineDiagnostics(machine) {
  const { vm } = machine;
  return `pc=0x${BigInt.asUintN(64, vm.virtPc()).toString(16)} ` +
    `insns=${vm.virtInsnCount()} jit-retired=${vm.ex.jit_stat(0)} ` +
    `jit-dispatches=${vm.ex.jit_stat(1)} jit-entries=${vm.ex.jit_stat(3)} ` +
    `tail=${JSON.stringify(output(machine).slice(-300))}`;
}
