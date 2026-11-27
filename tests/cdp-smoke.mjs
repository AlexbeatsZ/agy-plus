import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const electron = process.env.AGY_PLUS_TEST_ELECTRON;
const launcher = process.env.AGY_PLUS_TEST_LAUNCHER;
assert.ok(electron && launcher, "Set AGY_PLUS_TEST_ELECTRON and AGY_PLUS_TEST_LAUNCHER to existing binaries");
const temporaryRoot = process.platform === "win32"
  ? path.join(process.env.LOCALAPPDATA, "Temp", ".agents") : "/tmp/.agents";
await mkdir(temporaryRoot, { recursive: true });
const temporary = await mkdtemp(path.join(temporaryRoot, "agy-plus-injection-"));
const fixturePath = path.join(temporary, "main.cjs");
const profile = path.join(temporary, "profile");
let appProcess;
let launcherProcess;
let socket;
const failures = [];

async function until(check, label, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      const value = await check();
      if (value) return value;
    } catch {}
    await delay(100);
  }
  throw new Error(`Timed out: ${label}`);
}

try {
  await writeFile(fixturePath, `
const {app, BrowserWindow} = require('electron');
const http = require('node:http');
app.setPath('userData', ${JSON.stringify(profile)});
if (process.platform === 'darwin') app.setActivationPolicy('accessory');
const login = '<div class="entrance-auth-panel"><div><div id="other-sign-in-options"><button>Continue with Google</button></div></div></div>';
const settings = '<main><h2>Account</h2><div class="agy-card"><div><div><div><span>Not signed in</span></div></div><div><button>Sign In</button></div></div></div></main>';
app.whenReady().then(() => {
  const server = http.createServer((req, res) => {
    res.setHeader('Content-Type', 'text/html');
    if (req.url === '/child') { res.end('<!doctype html><html><head></head><body>Plugin fixture</body></html>'); return; }
    const body = req.url.startsWith('/settings') ? settings : login;
    res.end('<!doctype html><html><head><title>AGY injection fixture</title></head><body>' + body + '<iframe src="/child"></iframe></body></html>');
  });
  server.listen(0, '127.0.0.1', () => {
    const win = new BrowserWindow({show:false, webPreferences:{backgroundThrottling:false}});
    win.loadURL('http://127.0.0.1:' + server.address().port + '/login');
  });
});
app.on('window-all-closed', () => app.quit());
`);
  appProcess = spawn(electron, [fixturePath, `--user-data-dir=${profile}`, "--remote-debugging-port=0"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  appProcess.stderr.on("data", () => {});
  const port = await until(async () => Number((await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]), "fixture CDP port");
  const target = await until(async () => (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === "page" && t.url.startsWith("http://127.0.0.1:")), "fixture target");
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await once(socket, "open");
  let sequence = 0;
  const requests = new Map();
  socket.addEventListener("message", event => {
    const message = JSON.parse(event.data);
    if (message.id) requests.get(message.id)?.(message);
  });
  async function command(method, params = {}) {
    const id = ++sequence;
    let timer;
    const response = new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`CDP timeout: ${method}`)), 5000);
      requests.set(id, resolve);
    });
    socket.send(JSON.stringify({ id, method, params }));
    try {
      const result = await response;
      assert.equal(result.error, undefined, JSON.stringify(result.error));
      return result.result;
    } finally {
      clearTimeout(timer);
      requests.delete(id);
    }
  }
  async function evaluate(expression) {
    const result = await command("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  await command("Page.enable");
  await command("Page.addScriptToEvaluateOnNewDocument", {
    source: "window.__smokeErrors = []; window.addEventListener('error', e => window.__smokeErrors.push(e.message));",
  });
  launcherProcess = spawn(launcher, [`--attach-existing-port=${port}`], {
    env: { ...process.env, AGY_PLUS_HOME: path.join(temporary, "home"), AGY_PLUS_STATE_DIR: path.join(temporary, "state"), AGY_PLUS_TEST_KEYRING_FILE: path.join(temporary, "keyring.json"), AGY_PLUS_NO_DIALOG: "1" },
    stdio: ["ignore", "ignore", "pipe"],
  });
  launcherProcess.stderr.on("data", chunk => failures.push(chunk.toString()));
  await until(() => evaluate("!!document.querySelector('[data-agy-plus-login-rt]')"), "initial login injection");
  const baseline = process.argv.includes("--baseline");
  if (baseline) {
    await command("Page.reload");
    await until(() => evaluate("document.readyState === 'complete' && Array.isArray(window.__smokeErrors)"), "baseline reload");
    console.log("Baseline after reload:", await evaluate("({errors:window.__smokeErrors,api:typeof window.__agyPlus,bridge:typeof window.agyPlusInvoke,button:!!document.querySelector('[data-agy-plus-login-rt]')})"));
    assert.equal(await evaluate("!!document.querySelector('[data-agy-plus-login-rt]')"), false);
    await delay(16000);
    assert.equal(await evaluate("!!document.querySelector('[data-agy-plus-login-rt]')"), false);
    console.log("REPRODUCED: 0.2.0 loses injection after real Chromium reload and heartbeat does not recover it");
  } else {
    for (let round = 0; round < 4; round++) {
      const route = round % 2 ? "settings" : "login";
      const attribute = round % 2 ? "data-agy-plus-settings-rt" : "data-agy-plus-login-rt";
      await command("Page.navigate", { url: new URL(`/${route}?round=${round}`, target.url).href });
      await until(() => evaluate(`location.search === '?round=${round}' && !!document.querySelector('[${attribute}]')`), `navigation ${round}`, 3000);
      assert.deepEqual(await evaluate("window.__smokeErrors"), []);
      const result = await evaluate("window.agyPlusInvoke('/accounts/list', {})");
      assert.equal(result.status, "ok");
      assert.deepEqual(result.accounts, []);
    }
    await until(() => evaluate("!!document.querySelector('iframe')?.contentDocument?.body"), "child frame loaded");
    assert.equal(await evaluate("typeof document.querySelector('iframe').contentWindow.__agyPlus"), "undefined");
    await evaluate("window.__agyPlus.destroy(); delete window.__agyPlus; delete window.agyPlusInvoke; delete window.__agyPlusBridgeResolve");
    await until(() => evaluate("!!window.__agyPlus?.health?.().bridgeReady && !!document.querySelector('[data-agy-plus-settings-rt]')"), "watchdog reinjection after scripts disappear", 22000);
    assert.equal((await evaluate("window.agyPlusInvoke('/accounts/list', {})")).status, "ok");
    console.log("PASS: four login/settings navigations, parser startup, bridge after reload, iframe exclusion, lost-script watchdog recovery");
  }
} catch (error) {
  console.error(failures.join(""));
  throw error;
} finally {
  socket?.close();
  for (const child of [launcherProcess, appProcess]) {
    if (child && child.exitCode === null) {
      child.kill();
      await Promise.race([once(child, "exit"), delay(3000)]);
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }
  }
  await rm(temporary, { recursive: true, force: true });
}
