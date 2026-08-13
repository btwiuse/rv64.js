(port) => {
  const pending = new Map();
  const queue = [];
  let activeTag = null;
  const tagOf = (message) => message[5] | (message[6] << 8);

  const pump = () => {
    if (activeTag !== null) return;
    const entry = queue.shift();
    if (!entry) return;
    activeTag = entry.tag;
    try {
      port.postMessage(entry.request);
    } catch (error) {
      activeTag = null;
      pending.delete(entry.tag);
      entry.reject(error);
      queueMicrotask(pump);
    }
  };

  port.onmessage = ({ data }) => {
    const tag = tagOf(data);
    const entry = pending.get(tag);
    if (!entry || activeTag !== tag) return;
    pending.delete(tag);
    activeTag = null;
    entry.resolve(data);
    pump();
  };

  return (request) => new Promise((resolve, reject) => {
    const tag = tagOf(request);
    if (pending.has(tag)) {
      reject(new Error("9P tag collision: " + tag));
      return;
    }
    const entry = { request, tag, resolve, reject };
    pending.set(tag, entry);
    queue.push(entry);
    pump();
  });
}
