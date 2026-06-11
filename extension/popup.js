let activeTab = null;
let cookieText = '';
const hasChromeApi = typeof chrome !== 'undefined' && Boolean(chrome.runtime?.id);
const BADGE_STORAGE_KEY = 'tmwdShowPageBadge';

document.addEventListener('DOMContentLoaded', async () => {
  bindActions();
  setHint('Inspecting active tab...');
  if (!hasChromeApi) {
    renderPreviewState();
    return;
  }
  chrome.storage.onChanged.addListener(handleStorageChange);
  await Promise.all([loadActiveTab(), loadBadgeToggle()]);
  await Promise.all([probeBridge(), fetchCookies()]);
});

function bindActions() {
  document.getElementById('refresh').addEventListener('click', fetchCookies);
  document.getElementById('copyCookies').addEventListener('click', copyCookies);
  document.getElementById('copyUrl').addEventListener('click', copyUrl);
  document.getElementById('badgeToggle').addEventListener('change', updateBadgeToggle);
}

async function loadActiveTab() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  activeTab = tab || null;
  const url = activeTab?.url || '';
  document.getElementById('tabTitle').textContent = activeTab?.title || '-';
  document.getElementById('tabOrigin').textContent = originLabel(url);
  document.getElementById('tabId').textContent = activeTab?.id ?? '-';
  document.getElementById('runtimeId').textContent = chrome.runtime.id;
}

function renderPreviewState() {
  activeTab = null;
  document.getElementById('tabTitle').textContent = 'Preview unavailable outside extension';
  document.getElementById('tabOrigin').textContent = '-';
  document.getElementById('tabId').textContent = '-';
  document.getElementById('runtimeId').textContent = '-';
  document.getElementById('bridgeStatus').dataset.state = 'bad';
  document.getElementById('bridgeStatusText').textContent = 'Preview';
  document.getElementById('out').textContent = 'Open this popup from the installed extension to read the active tab cookies.';
  document.getElementById('badgeToggle').disabled = true;
  document.getElementById('badgeState').textContent = 'Debug badge is available inside the installed extension.';
  setHint('Static preview mode.');
}

function ensureChromeApi() {
  if (hasChromeApi) return true;
  renderPreviewState();
  return false;
}

async function probeBridge() {
  if (!ensureChromeApi()) return;
  const pill = document.getElementById('bridgeStatus');
  const text = document.getElementById('bridgeStatusText');
  pill.dataset.state = 'pending';
  text.textContent = 'Checking';

  try {
    const resp = await chrome.runtime.sendMessage({ cmd: 'bridgeHealth' });
    if (!resp?.ok) throw new Error(resp?.error || 'Bridge health unavailable');
    const body = resp.data;
    pill.dataset.state = body.extension_connected ? 'ok' : 'pending';
    text.textContent = body.extension_connected ? 'Connected' : 'Bridge ready';
  } catch (err) {
    pill.dataset.state = 'bad';
    text.textContent = 'Offline';
  }
}

async function fetchCookies() {
  if (!ensureChromeApi()) return;
  const out = document.getElementById('out');
  const copyButton = document.getElementById('copyCookies');
  cookieText = '';
  copyButton.disabled = true;
  setCookieCount(0);

  try {
    if (!activeTab?.url) {
      out.textContent = 'No active tab URL is available.';
      setHint('Open an HTTP or HTTPS page first.');
      return;
    }
    if (!/^https?:\/\//.test(activeTab.url)) {
      out.textContent = 'Cookies are only available for HTTP and HTTPS pages.';
      setHint('Internal browser pages are not scriptable.');
      return;
    }

    setHint('Reading cookies...');
    const resp = await chrome.runtime.sendMessage({ cmd: 'cookies', url: activeTab.url });
    if (!resp?.ok) {
      out.textContent = `Error: ${resp?.error || 'unknown'}`;
      setHint('Cookie read failed.');
      return;
    }
    const cookies = resp.data || [];
    setCookieCount(cookies.length);
    if (!cookies.length) {
      out.textContent = '(no cookies)';
      setHint('No cookies for this page.');
      return;
    }

    out.textContent = cookies.map(formatCookieLine).join('\n');
    cookieText = cookies.map(c => `${c.name}=${c.value}`).join('; ');
    copyButton.disabled = false;
    await copyText(cookieText, 'Cookie header copied.', 'Cookies loaded. Copy manually if needed.');
  } catch (err) {
    out.textContent = `Error: ${err.message || String(err)}`;
    setHint('Unexpected popup error.');
  }
}

async function copyCookies() {
  if (!cookieText) return;
  await copyText(cookieText, 'Cookie header copied.', 'Clipboard unavailable.');
}

async function copyUrl() {
  if (!activeTab?.url) return;
  await copyText(activeTab.url, 'URL copied.', 'Clipboard unavailable.');
}

async function loadBadgeToggle() {
  const store = await chrome.storage.local.get(BADGE_STORAGE_KEY);
  const enabled = Boolean(store[BADGE_STORAGE_KEY]);
  setBadgeToggle(enabled);
}

async function updateBadgeToggle(event) {
  const enabled = Boolean(event.target.checked);
  await chrome.storage.local.set({ [BADGE_STORAGE_KEY]: enabled });
  setBadgeToggle(enabled);
  await notifyActiveTabBadge(enabled);
  setHint(enabled ? 'Debug badge enabled.' : 'Debug badge disabled.');
}

function handleStorageChange(changes, areaName) {
  if (areaName !== 'local' || !changes[BADGE_STORAGE_KEY]) return;
  setBadgeToggle(Boolean(changes[BADGE_STORAGE_KEY].newValue));
}

function setBadgeToggle(enabled) {
  document.getElementById('badgeToggle').checked = enabled;
  document.getElementById('badgeState').textContent = enabled
    ? 'On for scriptable pages. Hide from the page badge or switch off here.'
    : 'Off by default. Shows a small diagnostic badge on pages.';
}

async function notifyActiveTabBadge(enabled) {
  if (!activeTab?.id || !/^https?:\/\//.test(activeTab.url || '')) return;
  try {
    await chrome.tabs.sendMessage(activeTab.id, { cmd: 'tmwdBadgeState', enabled });
  } catch (_) {
    // The page may not have this content script yet; storage still applies after reload.
  }
}

async function copyText(text, successHint, failureHint) {
  try {
    await navigator.clipboard.writeText(text);
    setHint(successHint);
    return true;
  } catch (_) {
    setHint(failureHint);
    return false;
  }
}

function formatCookieLine(cookie) {
  const flags = [
    cookie.httpOnly ? 'H' : '',
    cookie.secure ? 'S' : '',
    cookie.partitionKey ? 'P' : '',
  ].filter(Boolean);
  const suffix = flags.length ? ` [${flags.join('')}]` : '';
  return `${cookie.name}=${cookie.value}${suffix}`;
}

function originLabel(url) {
  try {
    return new URL(url).origin;
  } catch (_) {
    return url || '-';
  }
}

function setHint(text) {
  document.getElementById('hint').textContent = text;
}

function setCookieCount(count) {
  document.getElementById('cookieCount').textContent = `${count} ${count === 1 ? 'cookie' : 'cookies'}`;
}
