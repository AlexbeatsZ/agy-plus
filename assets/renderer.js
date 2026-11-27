(() => {
  "use strict";
  const String = window.String;

  const VERSION = 2;
  const STYLE_ID = "agy-plus-style";
  const MODAL_ID = "agy-plus-modal";
  const LOGIN_RT_ATTR = "data-agy-plus-login-rt";
  const ACCOUNT_SWITCH_ATTR = "data-agy-plus-switch-account";
  const SETTINGS_RT_ATTR = "data-agy-plus-settings-rt";

  if (window.__agyPlus?.version === VERSION) {
    window.__agyPlus.install();
    return;
  }
  window.__agyPlus?.destroy?.();

  const state = {
    observer: null,
    destroyed: false,
    scanQueued: false,
    modal: null,
    opener: null,
    signedOut: false,
    activeEmail: "",
    view: "accounts", // "accounts" | "rt"
    accounts: [],
    loading: false,
    busy: "",
    confirmDeleteId: "",
    confirmGoogleLogin: false,
    rtDraft: "",
    rtVisible: false,
    message: "",
    messageType: "",
    generation: 0,
  };

  function isRemoteWindow() {
    const cfg = window.__APP_CONFIG__;
    return Boolean(cfg && (cfg.isRemote || cfg.sshHost));
  }

  function normText(node) {
    return String(node?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  async function invoke(path, payload = {}, options = {}) {
    if (typeof window.agyPlusInvoke !== "function") {
      throw new Error("AGY Plus bridge is not ready yet.");
    }
    const result = await window.agyPlusInvoke(path, payload, options);
    if (["failed", "error"].includes(String(result?.status || "").toLowerCase())) {
      throw new Error(result?.message || "Operation failed.");
    }
    return result;
  }

  function keyIcon(size = 15) {
    return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><circle cx="7.5" cy="15.5" r="4.5"></circle><path d="m21 2-9.6 9.6"></path><path d="m15.5 7.5 3 3L22 7l-3-3"></path></svg>`;
  }

  function switchIcon(size = 14) {
    return `<svg aria-hidden="true" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path><circle cx="9" cy="7" r="4"></circle><path d="M22 21v-2a4 4 0 0 0-3-3.87"></path><path d="M16 3.13a4 4 0 0 1 0 7.75"></path></svg>`;
  }

  function closeIcon() {
    return `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"></path><path d="m6 6 12 12"></path></svg>`;
  }

  function eyeIcon(visible) {
    if (visible) {
      return `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9.88 9.88a3 3 0 1 0 4.24 4.24"></path><path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68"></path><path d="M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61"></path><line x1="2" x2="22" y1="2" y2="22"></line></svg>`;
    }
    return `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path><circle cx="12" cy="12" r="3"></circle></svg>`;
  }

  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;
    // New-document scripts run before the parser has created <html> or <head>.
    // The document observer will retry when the head is mounted.
    if (!document.head) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      .agy-plus-account-actions {
        display: inline-flex !important;
        align-items: center !important;
        gap: 8px !important;
        flex-wrap: nowrap !important;
      }
      .agy-plus-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        border: 1px solid transparent;
        font-family: var(--font-sans, inherit);
        font-weight: 500;
        line-height: 1;
        white-space: nowrap;
        user-select: none;
        cursor: pointer;
        outline: none;
        transition: background-color 140ms ease, color 140ms ease, border-color 140ms ease, opacity 140ms ease;
      }
      .agy-plus-btn:disabled {
        opacity: 0.5;
        cursor: not-allowed;
      }
      .agy-plus-btn:focus-visible {
        box-shadow: 0 0 0 2px var(--primary, #007acc);
      }
      .agy-plus-btn-sm {
        height: 28px;
        padding: 0 10px;
        border-radius: 6px;
        font-size: 13px;
      }
      .agy-plus-btn-lg {
        height: 36px;
        padding: 0 12px;
        border-radius: 8px;
        font-size: 14px;
        gap: 8px;
      }
      .agy-plus-btn-block {
        width: 100%;
        flex-shrink: 0;
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
      }
      .agy-plus-btn-primary {
        background: var(--primary, #007acc);
        color: var(--primary-foreground, #ffffff);
      }
      .agy-plus-btn-primary:hover:not(:disabled) {
        opacity: 0.9;
      }
      .agy-plus-btn-secondary {
        background: var(--secondary, #272727);
        color: var(--secondary-foreground, #b3b3b3);
      }
      .agy-plus-btn-secondary:hover:not(:disabled) {
        color: var(--foreground, #e5e5e5);
      }
      .agy-plus-btn-outline {
        background: transparent;
        border-color: var(--border, #2e2e2e);
        color: var(--secondary-foreground, #b3b3b3);
      }
      .agy-plus-btn-outline:hover:not(:disabled) {
        background: var(--secondary, #272727);
        color: var(--foreground, #e5e5e5);
      }
      .agy-plus-btn-ghost {
        background: transparent;
        color: var(--muted-foreground, #8a8a8a);
      }
      .agy-plus-btn-ghost:hover:not(:disabled) {
        background: var(--secondary, #272727);
        color: var(--foreground, #e5e5e5);
      }
      .agy-plus-btn-danger {
        background: transparent;
        color: var(--error, #de5555);
      }
      .agy-plus-btn-danger:hover:not(:disabled) {
        background: color-mix(in srgb, var(--error, #de5555) 14%, transparent);
      }
      .agy-plus-btn-danger-solid {
        background: var(--error, #de5555);
        color: #ffffff;
      }
      .agy-plus-btn-danger-solid:hover:not(:disabled) {
        opacity: 0.9;
      }

      .agy-plus-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483000;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 20px;
        background: var(--scrim, rgba(0, 0, 0, 0.45));
        backdrop-filter: blur(2px);
        font-family: var(--font-sans, system-ui, -apple-system, sans-serif);
      }
      .agy-plus-dialog {
        width: min(500px, calc(100vw - 32px));
        max-height: min(640px, calc(100vh - 32px));
        display: flex;
        flex-direction: column;
        overflow: hidden;
        background: var(--card, #191919);
        color: var(--foreground, #cccccc);
        border: 1px solid var(--card-border, var(--border, #2b2b2b));
        border-radius: 12px;
        box-shadow: 0 20px 56px rgba(0, 0, 0, 0.42);
        animation: agyPlusScaleIn 140ms cubic-bezier(0.16, 1, 0.3, 1);
      }
      @keyframes agyPlusScaleIn {
        from { opacity: 0; transform: scale(0.97); }
        to { opacity: 1; transform: scale(1); }
      }
      .agy-plus-header {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
        padding: 16px 18px 12px;
      }
      .agy-plus-title {
        margin: 0;
        font-size: 16px;
        line-height: 22px;
        font-weight: 500;
        color: var(--foreground, #e5e5e5);
      }
      .agy-plus-subtitle {
        margin: 3px 0 0;
        font-size: 13px;
        line-height: 18px;
        color: var(--muted-foreground, #8a8a8a);
      }
      .agy-plus-close {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        flex-shrink: 0;
        border: 0;
        border-radius: 6px;
        background: transparent;
        color: var(--muted-foreground, #8a8a8a);
        cursor: pointer;
      }
      .agy-plus-close:hover {
        background: var(--secondary, #272727);
        color: var(--foreground, #e5e5e5);
      }
      .agy-plus-body {
        padding: 4px 18px 18px;
        overflow-y: auto;
        display: flex;
        flex-direction: column;
        gap: 12px;
      }
      .agy-plus-status {
        border: 1px solid var(--border, #2b2b2b);
        border-radius: 8px;
        padding: 9px 12px;
        font-size: 13px;
        line-height: 18px;
        background: var(--muted, #151515);
        color: var(--muted-foreground, #9d9d9d);
      }
      .agy-plus-status[data-type="error"] {
        border-color: color-mix(in srgb, var(--error, #de5555) 45%, var(--border, #2b2b2b));
        color: var(--error, #de5555);
      }
      .agy-plus-status[data-type="success"] {
        border-color: color-mix(in srgb, var(--success, #7aae66) 45%, var(--border, #2b2b2b));
        color: var(--success, #7aae66);
      }
      .agy-plus-card {
        border: 1px solid var(--border, #2b2b2b);
        border-radius: 12px;
        overflow: hidden;
        background: transparent;
      }
      .agy-plus-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 12px;
        border-bottom: 1px solid var(--border, #2b2b2b);
      }
      .agy-plus-row:last-child {
        border-bottom: 0;
      }
      .agy-plus-row[data-active="true"] {
        background: color-mix(in srgb, var(--primary, #007acc) 7%, transparent);
      }
      .agy-plus-account-info {
        display: flex;
        align-items: center;
        gap: 10px;
        min-width: 0;
        flex: 1;
      }
      .agy-plus-avatar {
        width: 30px;
        height: 30px;
        border-radius: 999px;
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 12px;
        font-weight: 600;
        background: var(--secondary, #272727);
        color: var(--foreground, #cccccc);
        border: 1px solid var(--border, #2b2b2b);
      }
      .agy-plus-row[data-active="true"] .agy-plus-avatar {
        background: color-mix(in srgb, var(--primary, #007acc) 18%, var(--secondary, #272727));
        border-color: color-mix(in srgb, var(--primary, #007acc) 45%, var(--border, #2b2b2b));
      }
      .agy-plus-account-text {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
      }
      .agy-plus-name-line {
        display: flex;
        align-items: center;
        gap: 6px;
        min-width: 0;
      }
      .agy-plus-name {
        font-size: 13px;
        font-weight: 500;
        color: var(--foreground, #e5e5e5);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .agy-plus-email {
        font-size: 12px;
        color: var(--muted-foreground, #8a8a8a);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .agy-plus-badge {
        display: inline-flex;
        align-items: center;
        padding: 1px 6px;
        border-radius: 999px;
        font-size: 10px;
        line-height: 15px;
        font-weight: 600;
        border: 1px solid var(--border, #2b2b2b);
        background: var(--secondary, #272727);
        color: var(--secondary-foreground, #9d9d9d);
        flex-shrink: 0;
      }
      .agy-plus-badge[data-kind="current"] {
        border-color: color-mix(in srgb, var(--primary, #007acc) 45%, transparent);
        background: color-mix(in srgb, var(--primary, #007acc) 16%, transparent);
        color: var(--primary, #007acc);
      }
      .agy-plus-row-actions {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-shrink: 0;
      }
      .agy-plus-empty {
        padding: 26px 14px;
        text-align: center;
        font-size: 13px;
        color: var(--muted-foreground, #8a8a8a);
      }
      .agy-plus-confirm-bar {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        padding: 10px 12px;
        border: 1px solid var(--border, #2b2b2b);
        border-radius: 10px;
        background: var(--muted, #151515);
        font-size: 12px;
        color: var(--foreground, #cccccc);
      }
      .agy-plus-footer {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        flex-wrap: wrap;
        padding-top: 2px;
      }
      .agy-plus-footer-group {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
      }
      .agy-plus-field {
        display: flex;
        flex-direction: column;
        gap: 6px;
      }
      .agy-plus-label {
        font-size: 13px;
        font-weight: 500;
        color: var(--foreground, #cccccc);
      }
      .agy-plus-hint {
        margin: 0;
        font-size: 12px;
        line-height: 17px;
        color: var(--muted-foreground, #8a8a8a);
      }
      .agy-plus-input-wrap {
        position: relative;
        display: flex;
        align-items: center;
      }
      .agy-plus-input {
        width: 100%;
        height: 36px;
        box-sizing: border-box;
        padding: 0 36px 0 12px;
        border-radius: 8px;
        border: 1px solid var(--border, #2b2b2b);
        background: var(--background, #101010);
        color: var(--foreground, #cccccc);
        font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
        font-size: 12px;
        outline: none;
        transition: border-color 140ms ease, box-shadow 140ms ease;
      }
      .agy-plus-input::placeholder {
        color: var(--muted-foreground, #6e6e6e);
        font-family: var(--font-sans, inherit);
      }
      .agy-plus-input:focus {
        border-color: var(--primary, #007acc);
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--primary, #007acc) 45%, transparent);
      }
      .agy-plus-toggle-eye {
        position: absolute;
        right: 6px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 26px;
        height: 26px;
        border: 0;
        border-radius: 6px;
        background: transparent;
        color: var(--muted-foreground, #8a8a8a);
        cursor: pointer;
      }
      .agy-plus-toggle-eye:hover {
        color: var(--foreground, #cccccc);
        background: var(--secondary, #272727);
      }
      .agy-plus-divider-label {
        display: flex;
        align-items: center;
        gap: 10px;
        margin-top: 4px;
        font-size: 12px;
        color: var(--muted-foreground, #8a8a8a);
      }
      .agy-plus-divider-label::before,
      .agy-plus-divider-label::after {
        content: "";
        flex: 1;
        height: 1px;
        background: var(--border, #2b2b2b);
      }
    `;
    document.head.appendChild(style);
  }

  function isInsideOwnUi(node) {
    return Boolean(node?.closest?.(`#${MODAL_ID}`));
  }

  function isGoogleContinueButton(btn) {
    return /^(Continue|Sign\s*in)\s+with\s+Google(\s*Last Used)?$/i.test(normText(btn));
  }

  function isPersonalLoginCard(card) {
    if (!card || isInsideOwnUi(card)) return false;
    const text = normText(card);
    if (text.includes("Business Account Sign In")) return false;
    return (
      Boolean(card.querySelector("#other-sign-in-options")) ||
      Boolean(card.querySelector('[data-testid="onboarding-other-options-toggle"]')) ||
      Array.from(card.querySelectorAll("button")).some(isGoogleContinueButton)
    );
  }

  function createLoginRtButton() {
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute(LOGIN_RT_ATTR, "true");
    button.className = "agy-plus-btn agy-plus-btn-secondary agy-plus-btn-lg agy-plus-btn-block";
    button.title = "Continue with Refresh Token";
    button.innerHTML = `${keyIcon(15)}<span>Continue with Refresh Token</span>`;
    button.addEventListener(
      "click",
      (event) => {
        event.preventDefault();
        event.stopPropagation();
        openModal(button, { signedOut: true, view: "rt" });
      },
      true
    );
    return button;
  }

  function scanLoginScreen() {
    // 1. Primary official Antigravity onboarding/login panel.
    const panels = Array.from(document.querySelectorAll(".entrance-auth-panel"));
    for (const panel of panels) {
      if (isInsideOwnUi(panel)) continue;
      const card = panel.firstElementChild;
      if (!card || !isPersonalLoginCard(card)) continue;
      if (card.querySelector(`[${LOGIN_RT_ATTR}]`)) continue;
      const optionsBox = card.querySelector("#other-sign-in-options");
      if (optionsBox) {
        optionsBox.appendChild(createLoginRtButton());
      } else {
        const toggle = card.querySelector('[data-testid="onboarding-other-options-toggle"]');
        if (toggle && toggle.parentElement === card) {
          card.insertBefore(createLoginRtButton(), toggle.nextSibling);
        } else {
          card.appendChild(createLoginRtButton());
        }
      }
    }

    // 2. Fallback for older/alternate layouts without .entrance-auth-panel.
    const googleButtons = Array.from(document.querySelectorAll("button")).filter(
      (btn) => !isInsideOwnUi(btn) && !btn.hasAttribute(LOGIN_RT_ATTR) && isGoogleContinueButton(btn)
    );
    for (const googleBtn of googleButtons) {
      if (googleBtn.closest(".entrance-auth-panel")) continue;
      const container = googleBtn.parentElement;
      if (!container || container.querySelector(`[${LOGIN_RT_ATTR}]`)) continue;
      const siblings = Array.from(container.children);
      const businessBtn = siblings.find((el) => /Use Business Account/i.test(normText(el)));
      const anchor = businessBtn || googleBtn;
      container.insertBefore(createLoginRtButton(), anchor.nextSibling);
    }
  }

  function hasAccountSectionContext(element) {
    for (let cur = element?.parentElement, depth = 0; cur && depth < 8; cur = cur.parentElement, depth += 1) {
      const headings = Array.from(cur.querySelectorAll("h1, h2, h3"));
      if (headings.some((h) => /^Account$/i.test(normText(h)))) {
        return true;
      }
      if (normText(cur).includes("Manage your plan, credentials, and general preferences.")) {
        return true;
      }
    }
    return false;
  }

  function findRowForActionButton(button) {
    for (let cur = button?.parentElement, depth = 0; cur && depth < 5; cur = cur.parentElement, depth += 1) {
      if (cur.children.length >= 2) {
        const left = cur.children[0];
        const right = cur.children[cur.children.length - 1];
        if (right && right.contains(button) && left && !left.contains(button)) {
          return { row: cur, left, right };
        }
      }
    }
    return null;
  }

  function extractRowLabel(leftColumn) {
    if (!leftColumn) return "";
    return normText(leftColumn.firstElementChild || leftColumn);
  }

  function extractRowEmail(leftColumn) {
    if (!leftColumn) return "";
    // Inspect leaf elements individually so adjacent React JSX spans ("Email" + "user@gmail.com")
    // never get concatenated without whitespace.
    const leaves = Array.from(leftColumn.querySelectorAll("span,p,div")).filter(
      (node) => node.children.length === 0
    );
    for (const node of leaves) {
      const text = normText(node);
      const exact = text.match(/^[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}$/i);
      if (exact) return exact[0];
    }
    for (const node of leaves) {
      const text = normText(node);
      const inline = text.match(/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/i);
      if (inline) return inline[0];
    }
    return "";
  }

  function createSwitchAccountButton(email) {
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute(ACCOUNT_SWITCH_ATTR, "true");
    button.className = "agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm";
    button.innerHTML = `${switchIcon(14)}<span>Switch Account</span>`;
    button.addEventListener(
      "click",
      (event) => {
        event.preventDefault();
        event.stopPropagation();
        openModal(button, { signedOut: false, view: "accounts", activeEmail: email });
      },
      true
    );
    return button;
  }

  function createSettingsRtButton() {
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute(SETTINGS_RT_ATTR, "true");
    button.className = "agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm";
    button.innerHTML = `${keyIcon(14)}<span>Sign in with RT</span>`;
    button.addEventListener(
      "click",
      (event) => {
        event.preventDefault();
        event.stopPropagation();
        openModal(button, { signedOut: true, view: "rt" });
      },
      true
    );
    return button;
  }

  function scanAccountSettings() {
    const buttons = Array.from(document.querySelectorAll("button"));
    for (const button of buttons) {
      if (isInsideOwnUi(button)) continue;
      if (button.hasAttribute(ACCOUNT_SWITCH_ATTR) || button.hasAttribute(SETTINGS_RT_ATTR)) {
        continue;
      }
      const label = normText(button);

      // Signed-in Settings -> Account -> Email row ("Sign Out" / "Signing Out...").
      if (/^Sign(ing)?\s+Out(\.{3})?$/i.test(label)) {
        const parts = findRowForActionButton(button);
        if (!parts) continue;
        const leftLabel = extractRowLabel(parts.left);
        if (!/^Email$/i.test(leftLabel) || !hasAccountSectionContext(parts.row)) continue;
        if (parts.right.querySelector(`[${ACCOUNT_SWITCH_ATTR}]`)) continue;
        parts.right.classList.add("agy-plus-account-actions");
        const email = extractRowEmail(parts.left);
        parts.right.insertBefore(createSwitchAccountButton(email), parts.right.firstChild);
        continue;
      }

      // Signed-out Settings -> Account -> "Not signed in" row ("Sign In").
      if (/^Sign\s+In$/i.test(label)) {
        const parts = findRowForActionButton(button);
        if (!parts) continue;
        const leftLabel = extractRowLabel(parts.left);
        if (!/^Not signed in$/i.test(leftLabel) || !hasAccountSectionContext(parts.row)) continue;
        if (parts.right.querySelector(`[${SETTINGS_RT_ATTR}]`)) continue;
        parts.right.classList.add("agy-plus-account-actions");
        parts.right.insertBefore(createSettingsRtButton(), parts.right.firstChild);
      }
    }
  }

  function runScan() {
    installStyle();
    if (isRemoteWindow()) return;
    scanLoginScreen();
    scanAccountSettings();
  }

  function mutationIsRelevant(records) {
    if (!records) return true;
    const selector = `#${STYLE_ID},.entrance-auth-panel,#other-sign-in-options,.agy-card,button,h1,h2,h3`;
    return records.some((record) => {
      const target = record.target?.nodeType === 3 ? record.target.parentElement : record.target;
      if (isInsideOwnUi(target)) return false;
      if (target?.nodeType === 1 && (target.matches?.("html,head") || target.matches?.(selector) || target.closest?.(selector))) {
        return true;
      }
      return [...(record.addedNodes || []), ...(record.removedNodes || [])].some((node) => {
        if (node.nodeType !== 1 && node.nodeType !== 11) return false;
        if (node.nodeType === 1 && isInsideOwnUi(node)) return false;
        return Boolean(node.matches?.("html,head") || node.matches?.(selector) || node.querySelector?.(selector));
      });
    });
  }

  function queueScan(records) {
    if (!mutationIsRelevant(records)) return;
    if (state.scanQueued) return;
    state.scanQueued = true;
    queueMicrotask(() => {
      state.scanQueued = false;
      if (state.destroyed) return;
      runScan();
    });
  }

  function setMessage(message, type = "") {
    state.message = String(message || "");
    state.messageType = type;
  }

  function messageMarkup() {
    if (!state.message) return "";
    return `<div class="agy-plus-status" data-type="${escapeHtml(state.messageType)}" role="status">${escapeHtml(state.message)}</div>`;
  }

  function avatarInitial(account) {
    const src = account?.name || account?.email || "A";
    return String(src).trim().slice(0, 1).toUpperCase() || "A";
  }

  function sourceLabel(source) {
    return String(source || "").toLowerCase() === "rt" ? "RT" : "Google";
  }

  function accountRowMarkup(account, compact = false) {
    const id = String(account?.recordId || "");
    const active = account?.active === true;
    const primary = account?.name || account?.email || "Antigravity Account";
    const secondary =
      account?.name && account?.email && account.name !== account.email ? account.email : "";
    const busy = Boolean(state.busy);
    const confirmingDelete = state.confirmDeleteId === id;

    let actions = "";
    if (confirmingDelete) {
      actions = `
        <button type="button" class="agy-plus-btn agy-plus-btn-danger-solid agy-plus-btn-sm" data-agy-confirm-delete="${escapeHtml(id)}" ${busy ? "disabled" : ""}>${state.busy === `delete:${id}` ? "Removing..." : "Confirm"}</button>
        <button type="button" class="agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm" data-agy-cancel-delete ${busy ? "disabled" : ""}>Cancel</button>
      `;
    } else if (compact) {
      actions = `
        <button type="button" class="agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm" data-agy-switch="${escapeHtml(id)}" ${busy ? "disabled" : ""}>${state.busy === `switch:${id}` ? "Signing in..." : "Sign In"}</button>
      `;
    } else {
      actions = `
        <button type="button" class="agy-plus-btn ${active ? "agy-plus-btn-secondary" : "agy-plus-btn-primary"} agy-plus-btn-sm" data-agy-switch="${escapeHtml(id)}" ${active || busy ? "disabled" : ""}>${state.busy === `switch:${id}` ? "Switching..." : active ? "Current" : "Switch"}</button>
        <button type="button" class="agy-plus-btn agy-plus-btn-danger agy-plus-btn-sm" data-agy-delete="${escapeHtml(id)}" ${active || busy ? "disabled" : ""} title="${active ? "Switch to another account before removing" : "Remove from saved accounts"}">Remove</button>
      `;
    }

    return `
      <div class="agy-plus-row" data-record-id="${escapeHtml(id)}" data-active="${active}">
        <div class="agy-plus-account-info">
          <div class="agy-plus-avatar" aria-hidden="true">${escapeHtml(avatarInitial(account))}</div>
          <div class="agy-plus-account-text">
            <div class="agy-plus-name-line">
              <span class="agy-plus-name">${escapeHtml(primary)}</span>
              ${active ? '<span class="agy-plus-badge" data-kind="current">Current</span>' : ""}
              <span class="agy-plus-badge">${escapeHtml(sourceLabel(account?.source))}</span>
            </div>
            ${secondary ? `<span class="agy-plus-email">${escapeHtml(secondary)}</span>` : ""}
          </div>
        </div>
        <div class="agy-plus-row-actions">${actions}</div>
      </div>
    `;
  }

  function renderAccountsView(body) {
    const busy = Boolean(state.busy);
    if (state.loading) {
      body.innerHTML = `${messageMarkup()}<div class="agy-plus-empty">Loading saved accounts...</div>`;
      return;
    }

    const list = state.accounts.length
      ? `<div class="agy-plus-card">${state.accounts.map((item) => accountRowMarkup(item, false)).join("")}</div>`
      : '<div class="agy-plus-card"><div class="agy-plus-empty">No saved accounts yet. Sign in with a Refresh Token or add another Google account.</div></div>';

    const confirmGoogle = state.confirmGoogleLogin
      ? `<div class="agy-plus-confirm-bar">
          <span>Save current account and restart Antigravity to sign in with Google?</span>
          <div class="agy-plus-row-actions">
            <button type="button" class="agy-plus-btn agy-plus-btn-primary agy-plus-btn-sm" data-agy-confirm-google ${busy ? "disabled" : ""}>${state.busy === "google" ? "Restarting..." : "Continue"}</button>
            <button type="button" class="agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm" data-agy-cancel-google ${busy ? "disabled" : ""}>Cancel</button>
          </div>
        </div>`
      : "";

    const googleButton = state.signedOut
      ? ""
      : `<button type="button" class="agy-plus-btn agy-plus-btn-outline agy-plus-btn-sm" data-agy-google-login ${busy ? "disabled" : ""}>Add Google Account</button>`;

    body.innerHTML = `
      ${messageMarkup()}
      ${list}
      ${confirmGoogle}
      <div class="agy-plus-footer">
        <div class="agy-plus-footer-group">
          <button type="button" class="agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm" data-agy-open-rt ${busy ? "disabled" : ""}>${keyIcon(13)}<span>Sign in with Refresh Token</span></button>
          ${googleButton}
        </div>
      </div>
    `;

    body.querySelectorAll("[data-agy-switch]").forEach((btn) => {
      btn.addEventListener("click", () => void switchAccount(btn.dataset.agySwitch));
    });
    body.querySelectorAll("[data-agy-delete]").forEach((btn) => {
      btn.addEventListener("click", () => {
        state.confirmDeleteId = btn.dataset.agyDelete || "";
        state.confirmGoogleLogin = false;
        setMessage("");
        renderModal();
      });
    });
    body.querySelector("[data-agy-cancel-delete]")?.addEventListener("click", () => {
      state.confirmDeleteId = "";
      renderModal();
    });
    body.querySelector("[data-agy-confirm-delete]")?.addEventListener("click", (event) => {
      void deleteAccount(event.currentTarget.dataset.agyConfirmDelete);
    });
    body.querySelector("[data-agy-open-rt]")?.addEventListener("click", () => {
      state.view = "rt";
      state.confirmDeleteId = "";
      state.confirmGoogleLogin = false;
      setMessage("");
      renderModal();
      state.modal?.querySelector("[data-agy-rt-input]")?.focus();
    });
    body.querySelector("[data-agy-google-login]")?.addEventListener("click", () => {
      state.confirmGoogleLogin = true;
      state.confirmDeleteId = "";
      setMessage("");
      renderModal();
    });
    body.querySelector("[data-agy-cancel-google]")?.addEventListener("click", () => {
      state.confirmGoogleLogin = false;
      renderModal();
    });
    body.querySelector("[data-agy-confirm-google]")?.addEventListener("click", () => {
      void startGoogleLogin();
    });
  }

  function renderRtView(body) {
    const busy = Boolean(state.busy);
    const submitting = state.busy === "rt-switch" || state.busy === "rt-save";
    const savedSection =
      state.signedOut && state.accounts.length > 0
        ? `
          <div class="agy-plus-divider-label">Or sign in with a saved account</div>
          <div class="agy-plus-card">${state.accounts.map((item) => accountRowMarkup(item, true)).join("")}</div>
        `
        : "";

    const backButton = state.signedOut
      ? ""
      : `<button type="button" class="agy-plus-btn agy-plus-btn-secondary agy-plus-btn-sm" data-agy-back ${busy ? "disabled" : ""}>Back</button>`;

    const saveOnlyButton = state.signedOut
      ? ""
      : `<button type="button" class="agy-plus-btn agy-plus-btn-outline agy-plus-btn-sm" data-agy-rt-save ${busy ? "disabled" : ""}>${state.busy === "rt-save" ? "Saving..." : "Save Only"}</button>`;

    const primaryLabel = state.signedOut
      ? submitting && state.busy === "rt-switch"
        ? "Signing in..."
        : "Sign In"
      : submitting && state.busy === "rt-switch"
        ? "Signing in..."
        : "Sign In & Switch";

    body.innerHTML = `
      ${messageMarkup()}
      <form class="agy-plus-field" data-agy-rt-form>
        <label class="agy-plus-label" for="agy-plus-rt-input">Refresh Token</label>
        <div class="agy-plus-input-wrap">
          <input
            id="agy-plus-rt-input"
            data-agy-rt-input
            class="agy-plus-input"
            type="${state.rtVisible ? "text" : "password"}"
            autocomplete="off"
            spellcheck="false"
            placeholder="Paste your Antigravity Refresh Token"
            value="${escapeHtml(state.rtDraft)}"
            ${busy ? "disabled" : ""}
          />
          <button type="button" class="agy-plus-toggle-eye" data-agy-rt-eye aria-label="${state.rtVisible ? "Hide token" : "Show token"}" ${busy ? "disabled" : ""}>${eyeIcon(state.rtVisible)}</button>
        </div>
        <p class="agy-plus-hint">Verified against Google OAuth, saved to your local account list, and synced with Antigravity &amp; agy CLI.</p>
        <div class="agy-plus-footer">
          <div>${backButton}</div>
          <div class="agy-plus-footer-group">
            ${saveOnlyButton}
            <button type="submit" class="agy-plus-btn agy-plus-btn-primary agy-plus-btn-sm" data-agy-rt-submit ${busy ? "disabled" : ""}>${primaryLabel}</button>
          </div>
        </div>
      </form>
      ${savedSection}
    `;

    const input = body.querySelector("[data-agy-rt-input]");
    input?.addEventListener("input", () => {
      state.rtDraft = input.value;
    });
    body.querySelector("[data-agy-rt-eye]")?.addEventListener("click", () => {
      state.rtVisible = !state.rtVisible;
      renderModal();
      const refreshed = state.modal?.querySelector("[data-agy-rt-input]");
      if (refreshed) {
        refreshed.focus();
        refreshed.setSelectionRange(refreshed.value.length, refreshed.value.length);
      }
    });
    body.querySelector("[data-agy-back]")?.addEventListener("click", () => {
      state.view = "accounts";
      setMessage("");
      renderModal();
    });
    body.querySelector("[data-agy-rt-form]")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void submitRt(true);
    });
    body.querySelector("[data-agy-rt-save]")?.addEventListener("click", () => {
      void submitRt(false);
    });
    body.querySelectorAll("[data-agy-switch]").forEach((btn) => {
      btn.addEventListener("click", () => void switchAccount(btn.dataset.agySwitch));
    });
  }

  function renderModal() {
    if (!state.modal) return;
    const titleEl = state.modal.querySelector("#agy-plus-modal-title");
    const subtitleEl = state.modal.querySelector("[data-agy-subtitle]");
    const body = state.modal.querySelector("[data-agy-body]");
    if (!body) return;

    if (state.view === "rt") {
      if (titleEl) titleEl.textContent = "Sign in with Refresh Token";
      if (subtitleEl) {
        subtitleEl.textContent = state.signedOut
          ? "Authenticate with an Antigravity Refresh Token or choose a saved account."
          : "Add an Antigravity Refresh Token account to your local switcher.";
      }
      renderRtView(body);
    } else {
      if (titleEl) titleEl.textContent = "Switch Account";
      if (subtitleEl) {
        subtitleEl.textContent =
          "Switch between previously signed-in Google and Refresh Token accounts.";
      }
      renderAccountsView(body);
    }
  }

  async function loadAccounts() {
    const generation = ++state.generation;
    state.loading = true;
    renderModal();
    try {
      const result = await invoke("/accounts/list", {
        activeEmail: state.activeEmail || undefined,
      });
      if (generation !== state.generation) return;
      state.accounts = Array.isArray(result?.accounts) ? result.accounts : [];
    } catch (error) {
      if (generation !== state.generation) return;
      setMessage(error?.message || String(error), "error");
    } finally {
      if (generation === state.generation) {
        state.loading = false;
        renderModal();
      }
    }
  }

  async function submitRt(switchNow) {
    if (state.busy) return;
    const input = state.modal?.querySelector("[data-agy-rt-input]");
    if (input) state.rtDraft = input.value;
    const token = state.rtDraft.trim();
    if (!token || /\s/.test(token)) {
      setMessage("Enter a single valid Antigravity Refresh Token.", "error");
      renderModal();
      state.modal?.querySelector("[data-agy-rt-input]")?.focus();
      return;
    }

    state.busy = switchNow ? "rt-switch" : "rt-save";
    setMessage("");
    renderModal();
    try {
      const result = await invoke(
        "/accounts/rt-login",
        { refreshToken: token, switchNow },
        { timeoutMs: 35000 }
      );
      state.rtDraft = "";
      state.rtVisible = false;
      if (result?.restarting) {
        setMessage(
          result?.message || "Refresh Token verified. Restarting Antigravity...",
          "success"
        );
        renderModal();
      } else {
        state.busy = "";
        state.view = "accounts";
        const label = result?.account?.email || "Account";
        setMessage(`${label} saved. Click Switch to activate it.`, "success");
        await loadAccounts();
      }
    } catch (error) {
      state.busy = "";
      setMessage(error?.message || String(error), "error");
      renderModal();
    }
  }

  async function switchAccount(recordId) {
    if (!recordId || state.busy) return;
    state.busy = `switch:${recordId}`;
    state.confirmDeleteId = "";
    state.confirmGoogleLogin = false;
    setMessage("");
    renderModal();
    try {
      const result = await invoke("/accounts/switch", { recordId });
      setMessage(result?.message || "Switching account and restarting Antigravity...", "success");
      renderModal();
    } catch (error) {
      state.busy = "";
      setMessage(error?.message || String(error), "error");
      renderModal();
    }
  }

  async function deleteAccount(recordId) {
    if (!recordId || state.busy) return;
    state.busy = `delete:${recordId}`;
    setMessage("");
    renderModal();
    try {
      await invoke("/accounts/delete", {
        recordId,
        activeEmail: state.activeEmail || undefined,
      });
      state.accounts = state.accounts.filter((item) => String(item?.recordId || "") !== recordId);
      state.confirmDeleteId = "";
      setMessage("Removed saved account.", "success");
    } catch (error) {
      setMessage(error?.message || String(error), "error");
    } finally {
      state.busy = "";
      renderModal();
    }
  }

  async function startGoogleLogin() {
    if (state.busy) return;
    state.busy = "google";
    setMessage("");
    renderModal();
    try {
      const result = await invoke("/accounts/google-login", {});
      setMessage(
        result?.message || "Saving current account and restarting to Google sign-in...",
        "success"
      );
      renderModal();
    } catch (error) {
      state.busy = "";
      state.confirmGoogleLogin = false;
      setMessage(error?.message || String(error), "error");
      renderModal();
    }
  }

  function closeModal() {
    state.generation += 1;
    document.getElementById(MODAL_ID)?.remove();
    state.modal = null;
    state.busy = "";
    state.confirmDeleteId = "";
    state.confirmGoogleLogin = false;
    state.rtDraft = "";
    state.rtVisible = false;
    const opener = state.opener;
    state.opener = null;
    opener?.focus?.();
  }

  function focusable(root) {
    return Array.from(
      root.querySelectorAll(
        'button:not(:disabled),input:not(:disabled),textarea:not(:disabled),[tabindex]:not([tabindex="-1"])'
      )
    );
  }

  function openModal(opener = null, options = {}) {
    closeModal();
    installStyle();
    state.opener = opener instanceof HTMLElement ? opener : document.activeElement;
    state.signedOut = options?.signedOut === true;
    state.activeEmail = String(options?.activeEmail || "");
    state.view = options?.view === "rt" ? "rt" : "accounts";
    state.accounts = [];
    state.message = "";
    state.messageType = "";

    const overlay = document.createElement("div");
    overlay.id = MODAL_ID;
    overlay.className = "agy-plus-overlay";
    overlay.innerHTML = `
      <section class="agy-plus-dialog" role="dialog" aria-modal="true" aria-labelledby="agy-plus-modal-title">
        <header class="agy-plus-header">
          <div>
            <h2 class="agy-plus-title" id="agy-plus-modal-title"></h2>
            <p class="agy-plus-subtitle" data-agy-subtitle></p>
          </div>
          <button type="button" class="agy-plus-close" aria-label="Close">${closeIcon()}</button>
        </header>
        <div class="agy-plus-body" data-agy-body aria-live="polite"></div>
      </section>
    `;
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay || event.target.closest(".agy-plus-close")) {
        closeModal();
      }
    });
    overlay.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeModal();
        return;
      }
      if (event.key !== "Tab") return;
      const nodes = focusable(overlay);
      if (!nodes.length) return;
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    });

    document.body.appendChild(overlay);
    state.modal = overlay;
    renderModal();
    void loadAccounts();
    if (state.view === "rt") {
      overlay.querySelector("[data-agy-rt-input]")?.focus();
    }
  }

  function install() {
    state.destroyed = false;
    if (!state.observer) {
      state.observer = new MutationObserver((records) => queueScan(records));
    }
    // Document remains stable across parser startup and documentElement replacement.
    // Re-observing is idempotent and also repairs a disconnected observer on heartbeat.
    state.observer.observe(document, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    runScan();
    return health();
  }

  function health() {
    return {
      ready: document.getElementById(STYLE_ID) != null,
      observerReady: state.observer != null,
      bridgeReady: typeof window.agyPlusInvoke === "function" &&
        typeof window.agyPlusBridgeV1 === "function",
      documentLoading: document.readyState === "loading",
      modalOpen: document.getElementById(MODAL_ID) != null,
      version: VERSION,
    };
  }

  function destroy() {
    state.destroyed = true;
    closeModal();
    state.observer?.disconnect();
    state.observer = null;
    document
      .querySelectorAll(`[${LOGIN_RT_ATTR}],[${ACCOUNT_SWITCH_ATTR}],[${SETTINGS_RT_ATTR}]`)
      .forEach((node) => node.remove());
    document.getElementById(STYLE_ID)?.remove();
  }

  window.__agyPlus = {
    version: VERSION,
    install,
    health,
    destroy,
    open: openModal,
    state,
  };
  install();
})();
