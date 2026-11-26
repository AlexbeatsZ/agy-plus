(() => {
  "use strict";
  const bindingName = "__AGY_PLUS_BINDING_NAME__";
  const VERSION = 1;
  const DEFAULT_TIMEOUT_MS = 30000;

  // Reject any pending promises left over from a previous CDP session on reconnect.
  const previous = window.__agyPlusPending;
  if (previous && typeof previous.clear === "function") {
    for (const entry of Array.from(previous.values())) {
      try {
        entry.reject(new Error("AGY Plus bridge reconnected. Please retry."));
      } catch {}
    }
    previous.clear();
  }

  const pending = new Map();
  let sequence = 0;
  window.__agyPlusBridgeVersion = VERSION;
  window.__agyPlusPending = pending;

  window.agyPlusInvoke = (path, payload = {}, options = {}) =>
    new Promise((resolve, reject) => {
      const id = `${Date.now()}-${++sequence}-${Math.random().toString(16).slice(2)}`;
      const requested = Number(options?.timeoutMs);
      const limitMs =
        Number.isFinite(requested) && requested > 0 ? requested : DEFAULT_TIMEOUT_MS;
      const timer = setTimeout(() => {
        if (!pending.delete(id)) return;
        reject(new Error(`AGY Plus request timed out (${limitMs}ms): ${path}`));
      }, limitMs);
      pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      try {
        window[bindingName](JSON.stringify({ id, path, payload }));
      } catch (error) {
        pending.delete(id);
        clearTimeout(timer);
        reject(error);
      }
    });

  window.__agyPlusBridgeResolve = (id, value) => {
    const entry = pending.get(String(id));
    if (!entry) return false;
    pending.delete(String(id));
    entry.resolve(value);
    return true;
  };
})();
