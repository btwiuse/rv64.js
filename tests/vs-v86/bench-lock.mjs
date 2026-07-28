import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { rmSync } from "node:fs";
import { join } from "node:path";

function processAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
}

// Prevent two benchmark orchestrators from silently contaminating each other.
// Worker processes are children of the lock owner and do not acquire the lock.
export async function acquireBenchmarkLock(artifacts) {
  const path = join(artifacts, ".rv64-v86-benchmark.lock");
  const ownerPath = join(path, "owner.json");

  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      await mkdir(path);
      break;
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      let owner = null;
      try {
        owner = JSON.parse(await readFile(ownerPath, "utf8"));
      } catch {}
      if (owner && processAlive(owner.pid)) {
        throw new Error(
          `benchmark lock held by pid ${owner.pid} since ${owner.started}: ${path}`,
        );
      }
      // A dead owner left a stale lock. This path is deliberately exact and
      // scoped under ARTIFACTS; never recursively remove a broader target.
      await rm(path, { recursive: true, force: true });
    }
  }

  await writeFile(
    ownerPath,
    JSON.stringify({
      pid: process.pid,
      started: new Date().toISOString(),
      command: process.argv,
    }),
  );
  let released = false;
  const releaseSync = () => {
    if (released) return;
    released = true;
    try {
      rmSync(path, { recursive: true, force: true });
    } catch {}
  };
  process.once("exit", releaseSync);

  return async () => {
    if (released) return;
    released = true;
    process.removeListener("exit", releaseSync);
    await rm(path, { recursive: true, force: true });
  };
}
