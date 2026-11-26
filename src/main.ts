import { invoke, isTauri } from '@tauri-apps/api/core';
import './style.css';
const form = document.querySelector<HTMLFormElement>('#login-form')!;
const input = document.querySelector<HTMLInputElement>('#refresh-token')!;
const toggle = document.querySelector<HTMLButtonElement>('#toggle')!;
const submit = document.querySelector<HTMLButtonElement>('#submit')!;
const status = document.querySelector<HTMLElement>('#status')!;
let busy = false;
toggle.addEventListener('click', () => {
  const visible = input.type === 'password';
  input.type = visible ? 'text' : 'password';
  toggle.textContent = visible ? '隐藏' : '显示';
  toggle.setAttribute('aria-label', visible ? '隐藏 RT' : '显示 RT');
  toggle.setAttribute('aria-pressed', String(visible));
});
form.addEventListener('submit', async event => {
  event.preventDefault();
  if (busy) return;
  const rt = input.value.trim();
  status.dataset.kind = 'error';
  if (!rt || /\s/.test(rt)) { status.textContent = '请粘贴单个完整的 Refresh Token。'; return; }
  if (!isTauri()) { status.textContent = '请在 AGY Plus 桌面程序内登录。'; return; }
  busy = true; submit.disabled = true; input.disabled = true; toggle.disabled = true;
  status.dataset.kind = 'pending'; status.textContent = '正在验证 RT 并更新 agy 登录…';
  submit.textContent = '正在登录…';
  try {
    const result = await invoke<{email: string | null}>('login_with_rt', {refreshToken: rt});
    input.value = ''; input.type = 'password'; toggle.textContent = '显示';
    toggle.setAttribute('aria-pressed', 'false'); toggle.setAttribute('aria-label', '显示 RT');
    status.dataset.kind = 'success';
    status.textContent = result.email ? `已为 agy 写入登录凭据：${result.email}。请重新打开 CLI。` : '已为 agy 写入登录凭据，请重新打开 CLI。';
  } catch (error) { status.dataset.kind = 'error'; status.textContent = String(error); }
  finally { busy = false; submit.disabled = false; input.disabled = false; toggle.disabled = false; submit.innerHTML = '登录 agy <span aria-hidden="true">↗</span>'; }
});
