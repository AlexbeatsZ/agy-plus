import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { JSDOM } from "jsdom";

const bridgeScript = await readFile(new URL("../assets/bridge.js", import.meta.url), "utf8");
const rendererScript = await readFile(new URL("../assets/renderer.js", import.meta.url), "utf8");

async function flush(rounds = 4) {
  for (let i = 0; i < rounds; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function setupDom(bodyHtml, invoke, appConfig = {}) {
  const dom = new JSDOM(`<!doctype html><html><head></head><body>${bodyHtml}</body></html>`, {
    runScripts: "dangerously",
    url: "https://127.0.0.1:54321/",
  });
  dom.window.__APP_CONFIG__ = appConfig;
  dom.window.agyPlusInvoke = invoke;
  dom.window.eval(rendererScript);
  const close = dom.window.close.bind(dom.window);
  dom.window.close = () => {
    dom.window.__agyPlus?.destroy?.();
    close();
  };
  return dom;
}

const LOGIN_SCREEN_HTML = `
  <div class="entrance-auth-panel">
    <div class="relative border border-border bg-card rounded-2xl p-6 flex flex-col items-center gap-4">
      <div id="other-sign-in-options" class="self-stretch flex flex-col gap-4">
        <button type="button">Continue with Google</button>
        <button type="button">Use Business Account</button>
      </div>
    </div>
  </div>
`;

const ACCOUNT_SETTINGS_SIGNED_IN_HTML = `
  <main>
    <h2>Account</h2>
    <p>Manage your plan, credentials, and general preferences.</p>
    <div class="space-y-2">
      <h3>Account</h3>
      <div class="agy-card bg-transparent rounded-xl border border-border overflow-hidden divide-y divide-border">
        <div class="py-2 px-3 w-full flex justify-between gap-4 items-center">
          <div class="flex-1 flex flex-col gap-0.5">
            <div><span>Email</span></div>
            <span class="text-sm text-muted-foreground">alice@gmail.com</span>
          </div>
          <div class="shrink-0">
            <span data-tooltip-id="logout-tip">
              <button type="button">Sign Out</button>
            </span>
          </div>
        </div>
      </div>
    </div>
  </main>
`;

const ACCOUNT_SETTINGS_SIGNED_OUT_HTML = `
  <main>
    <h2>Account</h2>
    <p>Manage your plan, credentials, and general preferences.</p>
    <div class="space-y-2">
      <h3>Account</h3>
      <div class="agy-card">
        <div class="py-2 px-3 w-full flex justify-between gap-4 items-center">
          <div class="flex-1 flex flex-col gap-0.5">
            <div><span>Not signed in</span></div>
            <span class="text-sm text-muted-foreground">Sign in to use Antigravity!</span>
          </div>
          <div class="shrink-0">
            <button type="button">Sign In</button>
          </div>
        </div>
      </div>
    </div>
  </main>
`;

test("signed-out login screen injects Continue with Refresh Token button idempotently and submits RT", async () => {
  const calls = [];
  const dom = setupDom(LOGIN_SCREEN_HTML, async (path, payload) => {
    calls.push({ path, payload });
    if (path === "/accounts/list") {
      return {
        status: "ok",
        accounts: [
          {
            recordId: "a".repeat(64),
            email: "saved@gmail.com",
            name: "Saved User",
            source: "oauth",
            active: false,
          },
        ],
      };
    }
    if (path === "/accounts/rt-login") {
      return {
        status: "ok",
        restarting: true,
        message: "Refresh Token verified. Restarting Antigravity...",
      };
    }
    return { status: "ok" };
  });

  try {
    await flush();
    const box = dom.window.document.getElementById("other-sign-in-options");
    assert.deepEqual(
      Array.from(box.querySelectorAll("button")).map((b) => b.textContent.trim()),
      ["Continue with Google", "Use Business Account", "Continue with Refresh Token"]
    );

    // Re-running install must not duplicate the button.
    dom.window.__agyPlus.install();
    await flush();
    assert.equal(box.querySelectorAll("[data-agy-plus-login-rt]").length, 1);

    // Click opens RT modal directly AND shows saved accounts below.
    box.querySelector("[data-agy-plus-login-rt]").click();
    await flush();
    const modal = dom.window.document.getElementById("agy-plus-modal");
    assert.ok(modal);
    assert.equal(modal.querySelector("#agy-plus-modal-title").textContent, "Sign in with Refresh Token");
    assert.match(modal.textContent, /Or sign in with a saved account/);
    assert.match(modal.textContent, /saved@gmail\.com/);

    // Invalid multiline RT shows error without calling backend.
    const input = modal.querySelector("[data-agy-rt-input]");
    input.value = "bad token with spaces";
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    modal.querySelector("[data-agy-rt-submit]").click();
    await flush();
    assert.equal(calls.filter((c) => c.path === "/accounts/rt-login").length, 0);
    assert.match(modal.querySelector(".agy-plus-status").textContent, /Enter a single valid/);

    // Valid RT calls /accounts/rt-login with switchNow: true.
    const refreshedInput = modal.querySelector("[data-agy-rt-input]");
    refreshedInput.value = "  1//valid-rt-example  ";
    refreshedInput.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    modal.querySelector("[data-agy-rt-submit]").click();
    await flush();
    const rtCall = calls.find((c) => c.path === "/accounts/rt-login");
    assert.deepEqual(
      { ...rtCall.payload },
      { refreshToken: "1//valid-rt-example", switchNow: true }
    );
    assert.match(modal.querySelector(".agy-plus-status").textContent, /Restarting Antigravity/);
  } finally {
    dom.window.close();
  }
});

test("collapsed Last Used login card gets RT button while Business Account sub-view does not", async () => {
  const collapsedHtml = `
    <div class="entrance-auth-panel">
      <div>
        <button type="button">Continue with Google Cloud</button>
        <button type="button" data-testid="onboarding-other-options-toggle">Other sign-in options</button>
      </div>
    </div>
  `;
  const dom = setupDom(collapsedHtml, async () => ({ status: "ok", accounts: [] }));
  try {
    await flush();
    const btn = dom.window.document.querySelector("[data-agy-plus-login-rt]");
    assert.ok(btn);
    assert.equal(btn.textContent.trim(), "Continue with Refresh Token");
  } finally {
    dom.window.close();
  }

  const businessHtml = `
    <div class="entrance-auth-panel">
      <div>
        <div>Business Account Sign In</div>
        <button type="button">Continue with Google Cloud</button>
      </div>
    </div>
  `;
  const bizDom = setupDom(businessHtml, async () => ({ status: "ok", accounts: [] }));
  try {
    await flush();
    assert.equal(bizDom.window.document.querySelector("[data-agy-plus-login-rt]"), null);
  } finally {
    bizDom.window.close();
  }
});

test("signed-in modal supports Save Only RT, Switch, inline Google Login confirm, and ignores DOM churn", async () => {
  const calls = [];
  const dom = setupDom(`<main id="app"></main>`, async (path, payload) => {
    calls.push({ path, payload });
    if (path === "/accounts/list") {
      return {
        status: "ok",
        accounts: [
          { recordId: "1".repeat(64), email: "one@gmail.com", name: "One", source: "oauth", active: true },
          { recordId: "2".repeat(64), email: "two@gmail.com", name: "Two", source: "rt", active: false },
        ],
      };
    }
    if (path === "/accounts/rt-login") {
      return { status: "ok", restarting: false, account: { email: "three@gmail.com" } };
    }
    return { status: "ok", restarting: true };
  });

  try {
    await flush();
    // Unrelated DOM churn must not break or inject anything.
    const chat = dom.window.document.createElement("div");
    for (let i = 0; i < 40; i += 1) {
      const span = dom.window.document.createElement("span");
      span.textContent = `chunk ${i}`;
      chat.appendChild(span);
    }
    dom.window.document.getElementById("app").appendChild(chat);
    await flush();
    assert.equal(dom.window.document.querySelector("[data-agy-plus-switch-account]"), null);

    // Dynamically mount Account Settings -> observer injects Switch Account.
    dom.window.document.getElementById("app").innerHTML = ACCOUNT_SETTINGS_SIGNED_IN_HTML;
    await flush();
    const switchBtn = dom.window.document.querySelector("[data-agy-plus-switch-account]");
    assert.ok(switchBtn);
    switchBtn.click();
    await flush();

    // Open RT view and Save Only (switchNow: false).
    const modal = dom.window.document.getElementById("agy-plus-modal");
    modal.querySelector("[data-agy-open-rt]").click();
    await flush();
    const rtInput = modal.querySelector("[data-agy-rt-input]");
    rtInput.value = "1//save-only-token";
    rtInput.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    modal.querySelector("[data-agy-rt-save]").click();
    await flush();
    const rtCall = calls.find((c) => c.path === "/accounts/rt-login");
    assert.deepEqual({ ...rtCall.payload }, { refreshToken: "1//save-only-token", switchNow: false });

    // Trigger Add Google Account -> requires inline confirmation.
    modal.querySelector("[data-agy-google-login]").click();
    await flush();
    assert.equal(calls.filter((c) => c.path === "/accounts/google-login").length, 0);
    modal.querySelector("[data-agy-confirm-google]").click();
    await flush();
    assert.equal(calls.filter((c) => c.path === "/accounts/google-login").length, 1);
  } finally {
    dom.window.close();
  }
});

test("Settings -> Account injects Switch Account beside Sign Out, lists OAuth and RT accounts, switches and deletes", async () => {
  const calls = [];
  const dom = setupDom(ACCOUNT_SETTINGS_SIGNED_IN_HTML, async (path, payload) => {
    calls.push({ path, payload });
    if (path === "/accounts/list") {
      return {
        status: "ok",
        accounts: [
          {
            recordId: "1".repeat(64),
            email: "alice@gmail.com",
            name: "Alice",
            source: "oauth",
            active: true,
          },
          {
            recordId: "2".repeat(64),
            email: "bob@gmail.com",
            name: "Bob <RT>",
            source: "rt",
            active: false,
          },
        ],
      };
    }
    if (path === "/accounts/switch") {
      return { status: "ok", restarting: true, message: "Switching account..." };
    }
    if (path === "/accounts/delete") {
      return { status: "ok" };
    }
    return { status: "ok" };
  });

  try {
    await flush();
    const switchBtn = dom.window.document.querySelector("[data-agy-plus-switch-account]");
    assert.ok(switchBtn);
    assert.equal(switchBtn.textContent.trim(), "Switch Account");
    assert.equal(switchBtn.parentElement.querySelectorAll("button").length, 2);

    switchBtn.click();
    await flush();
    const listCall = calls.find((c) => c.path === "/accounts/list");
    assert.equal(listCall.payload.activeEmail, "alice@gmail.com");

    const modal = dom.window.document.getElementById("agy-plus-modal");
    const rows = modal.querySelectorAll(".agy-plus-row");
    assert.equal(rows.length, 2);
    // HTML escaping protects account display names.
    assert.equal(rows[1].querySelector(".agy-plus-name").textContent, "Bob <RT>");
    assert.match(rows[0].textContent, /Current/);
    assert.match(rows[0].textContent, /Google/);
    assert.match(rows[1].textContent, /RT/);
    assert.equal(rows[0].querySelector("[data-agy-delete]").disabled, true);

    // Inline two-step delete confirmation removes the inactive account.
    rows[1].querySelector("[data-agy-delete]").click();
    await flush();
    modal.querySelector("[data-agy-confirm-delete]").click();
    await flush();
    assert.equal(calls.find((c) => c.path === "/accounts/delete").payload.recordId, "2".repeat(64));
    assert.equal(modal.querySelectorAll(".agy-plus-row").length, 1);
  } finally {
    dom.window.close();
  }
});

test("Settings -> Account signed-out row gets Sign in with RT button and skips remote SSH windows", async () => {
  const dom = setupDom(ACCOUNT_SETTINGS_SIGNED_OUT_HTML, async () => ({
    status: "ok",
    accounts: [],
  }));
  try {
    await flush();
    const rtBtn = dom.window.document.querySelector("[data-agy-plus-settings-rt]");
    assert.ok(rtBtn);
    assert.equal(rtBtn.textContent.trim(), "Sign in with RT");
  } finally {
    dom.window.close();
  }

  const remoteDom = setupDom(
    ACCOUNT_SETTINGS_SIGNED_IN_HTML,
    async () => ({ status: "ok", accounts: [] }),
    { isRemote: true }
  );
  try {
    await flush();
    assert.equal(remoteDom.window.document.querySelector("[data-agy-plus-switch-account]"), null);
    assert.equal(remoteDom.window.__agyPlus.health().ready, true);
  } finally {
    remoteDom.window.close();
  }
});

test("unrelated buttons outside Account settings are not modified", async () => {
  const dom = setupDom(
    `<main><h2>Plugins</h2><div class="agy-card"><div><span>GitHub</span></div><div><button type="button">Sign Out</button></div></div></main>`,
    async () => ({ status: "ok", accounts: [] })
  );
  try {
    await flush();
    assert.equal(dom.window.document.querySelector("[data-agy-plus-switch-account]"), null);
  } finally {
    dom.window.close();
  }
});

test("handles compact zero-whitespace React JSX DOM for Account row and Last Used badge", async () => {
  const compactAccount =
    '<main><h2>Account</h2><h3>Account</h3><div class="agy-card"><div class="py-2 px-3 flex justify-between"><div><div><span>Email</span></div><span>alice@gmail.com</span></div><div class="shrink-0"><span><button type="button">Sign Out</button></span></div></div></div></main>';
  const calls = [];
  const dom = setupDom(compactAccount, async (path, payload) => {
    calls.push({ path, payload });
    return { status: "ok", accounts: [] };
  });
  try {
    await flush();
    const btn = dom.window.document.querySelector("[data-agy-plus-switch-account]");
    assert.ok(btn);
    btn.click();
    await flush();
    assert.equal(calls.find((c) => c.path === "/accounts/list").payload.activeEmail, "alice@gmail.com");
  } finally {
    dom.window.close();
  }

  const compactLogin =
    '<div class="entrance-auth-panel"><div><button type="button">Continue with Google<span data-testid="onboarding-last-used-badge">Last Used</span></button></div></div>';
  const loginDom = setupDom(compactLogin, async () => ({ status: "ok", accounts: [] }));
  try {
    await flush();
    assert.ok(loginDom.window.document.querySelector("[data-agy-plus-login-rt]"));
  } finally {
    loginDom.window.close();
  }
});

test("bridge script resolves responses and rejects pending requests on reconnect", async () => {
  const dom = new JSDOM(`<!doctype html><html><body></body></html>`, {
    runScripts: "dangerously",
    url: "https://127.0.0.1:54321/",
  });
  try {
    const sent = [];
    dom.window.agyPlusBridgeV1 = (raw) => sent.push(JSON.parse(raw));
    const script = bridgeScript.replace('"__AGY_PLUS_BINDING_NAME__"', '"agyPlusBridgeV1"');
    dom.window.eval(script);

    const p1 = dom.window.agyPlusInvoke("/accounts/list", {});
    assert.equal(sent.length, 1);
    dom.window.__agyPlusBridgeResolve(sent[0].id, { status: "ok", accounts: [] });
    assert.deepEqual(await p1, { status: "ok", accounts: [] });

    const p2 = dom.window.agyPlusInvoke("/accounts/list", {});
    dom.window.eval(script);
    await assert.rejects(p2, /reconnected/i);
  } finally {
    dom.window.close();
  }
});

test("preload installs before the parser creates html/head and survives login DOM mounting", async () => {
  const dom = setupDom("", async () => ({ status: "ok", accounts: [] }));
  try {
    dom.window.__agyPlus.destroy();
    delete dom.window.__agyPlus;
    dom.window.document.documentElement.remove();
    assert.doesNotThrow(() => dom.window.eval(rendererScript));
    const html = dom.window.document.createElement("html");
    html.innerHTML = `<head></head><body>${LOGIN_SCREEN_HTML}</body>`;
    dom.window.document.appendChild(html);
    await flush();
    assert.ok(dom.window.document.querySelector("[data-agy-plus-login-rt]"));
    assert.equal(dom.window.__agyPlus.health().ready, true);
    assert.equal(dom.window.__agyPlus.health().observerReady, true);
  } finally {
    dom.window.close();
  }
});

test("React removing only the injected account button repairs it without waiting for a reload", async () => {
  const dom = setupDom(ACCOUNT_SETTINGS_SIGNED_IN_HTML, async () => ({ status: "ok", accounts: [] }));
  try {
    await flush();
    const original = dom.window.document.querySelector("[data-agy-plus-switch-account]");
    original.remove();
    await flush();
    const repaired = dom.window.document.querySelector("[data-agy-plus-switch-account]");
    assert.ok(repaired);
    assert.notEqual(repaired, original);
    repaired.click();
    await flush();
    assert.ok(dom.window.document.getElementById("agy-plus-modal"));
  } finally {
    dom.window.close();
  }
});

test("replacing the document element reconnects observation and restores style and controls", async () => {
  const dom = setupDom(LOGIN_SCREEN_HTML, async () => ({ status: "ok", accounts: [] }));
  try {
    await flush();
    const html = dom.window.document.createElement("html");
    html.innerHTML = `<head></head><body>${ACCOUNT_SETTINGS_SIGNED_OUT_HTML}</body>`;
    dom.window.document.replaceChild(html, dom.window.document.documentElement);
    await flush();
    assert.ok(dom.window.document.querySelector("[data-agy-plus-settings-rt]"));
    assert.equal(dom.window.__agyPlus.health().ready, true);
  } finally {
    dom.window.close();
  }
});
