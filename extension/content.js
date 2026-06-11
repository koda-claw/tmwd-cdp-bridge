;(function(){
  if (/streamlit/i.test(document.title)) return;

  const BADGE_STORAGE_KEY = 'tmwdShowPageBadge';
  const BADGE_HOST_ID = '__tmwd_cdp_bridge_badge';
  const requestIds = Array.isArray(globalThis.TMWD_REQUEST_IDS)
    ? globalThis.TMWD_REQUEST_IDS
    : ['__tmwd_cdp_bridge_request', '__ljq_045ef1'];
  const selector = requestIds.map(id => `#${cssEscape(id)}`).join(',');
  let badgeHost = null;
  let badgeShadow = null;
  let badgeTimer = null;

  document.querySelectorAll('meta[http-equiv="Content-Security-Policy"]').forEach(e => e.remove());

  new MutationObserver(muts => {
    for (const m of muts) {
      for (const n of m.addedNodes) {
        for (const el of findRequestElements(n)) handle(el);
      }
    }
  }).observe(document.documentElement, { childList: true, subtree: true });
  document.querySelectorAll(selector).forEach(handle);
  initBadge();

  chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg?.cmd !== 'tmwdBadgeState') return false;
    setBadgeVisible(Boolean(msg.enabled));
    sendResponse({ ok: true });
    return false;
  });
  chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== 'local' || !changes[BADGE_STORAGE_KEY]) return;
    setBadgeVisible(Boolean(changes[BADGE_STORAGE_KEY].newValue));
  });

  function findRequestElements(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) return [];
    if (requestIds.includes(node.id)) return [node];
    return node.querySelectorAll ? Array.from(node.querySelectorAll(selector)) : [];
  }

  async function handle(el) {
    try {
      const req = el.textContent.trim() ? JSON.parse(el.textContent) : { cmd: 'cookies' };
      const cmd = req.cmd || 'cookies';
      let resp;
      if (cmd === 'cookies') {
        resp = await chrome.runtime.sendMessage({ cmd: 'cookies', url: req.url || location.href });
      } else if (cmd === 'cdp') {
        resp = await chrome.runtime.sendMessage({ cmd: 'cdp', method: req.method, params: req.params || {}, tabId: req.tabId });
      } else if (cmd === 'batch') {
        resp = await chrome.runtime.sendMessage({ cmd: 'batch', commands: req.commands, tabId: req.tabId });
      } else if (cmd === 'tabs') {
        resp = await chrome.runtime.sendMessage({ cmd: 'tabs', method: req.method, tabId: req.tabId });
      } else if (cmd === 'management') {
        resp = await chrome.runtime.sendMessage({ cmd: 'management', method: req.method, extId: req.extId, confirmSelf: req.confirmSelf });
      } else if (cmd === 'contentSettings') {
        resp = await chrome.runtime.sendMessage({ cmd: 'contentSettings', type: req.type, setting: req.setting, pattern: req.pattern });
      } else {
        resp = { ok: false, error: 'unknown cmd: ' + cmd };
      }
      el.textContent = JSON.stringify(resp);
    } catch (e) {
      el.textContent = JSON.stringify({ ok: false, error: e.message });
    }
  }

  function cssEscape(value) {
    if (globalThis.CSS?.escape) return CSS.escape(value);
    return String(value).replace(/[^a-zA-Z0-9_-]/g, '\\$&');
  }

  async function initBadge() {
    if (window.self !== window.top || !/^https?:/.test(location.href)) return;
    try {
      const store = await chrome.storage.local.get(BADGE_STORAGE_KEY);
      setBadgeVisible(Boolean(store[BADGE_STORAGE_KEY]));
    } catch (_) {
      setBadgeVisible(false);
    }
  }

  function setBadgeVisible(visible) {
    if (!visible) {
      removeBadge();
      return;
    }
    renderBadge();
    refreshBadgeState();
  }

  function renderBadge() {
    if (badgeHost) return;
    badgeHost = document.createElement('div');
    badgeHost.id = BADGE_HOST_ID;
    badgeHost.style.cssText = 'all:initial;position:fixed;right:12px;bottom:12px;z-index:2147483647;';
    badgeShadow = badgeHost.attachShadow({ mode: 'closed' });
    badgeShadow.innerHTML = `
      <style>
        :host { all: initial; }
        .wrap {
          position: relative;
          font: 12px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
          color: #edf4fb;
        }
        button {
          font: inherit;
        }
        .pill {
          display: inline-flex;
          align-items: center;
          gap: 6px;
          min-height: 26px;
          border: 1px solid rgba(150, 165, 185, .42);
          border-radius: 999px;
          padding: 4px 9px;
          background: rgba(15, 18, 24, .86);
          color: #edf4fb;
          box-shadow: 0 8px 24px rgba(0, 0, 0, .28);
          cursor: pointer;
        }
        .dot {
          width: 8px;
          height: 8px;
          border-radius: 50%;
          background: #f4c15d;
          box-shadow: 0 0 0 3px rgba(244, 193, 93, .16);
        }
        .wrap[data-state="connected"] .dot {
          background: #42d392;
          box-shadow: 0 0 0 3px rgba(66, 211, 146, .16);
        }
        .wrap[data-state="offline"] .dot {
          background: #ff6b6b;
          box-shadow: 0 0 0 3px rgba(255, 107, 107, .16);
        }
        .panel {
          display: none;
          position: absolute;
          right: 0;
          bottom: 34px;
          width: min(320px, calc(100vw - 24px));
          border: 1px solid rgba(150, 165, 185, .42);
          border-radius: 8px;
          background: rgba(17, 20, 27, .96);
          box-shadow: 0 12px 34px rgba(0, 0, 0, .34);
          overflow: hidden;
        }
        .wrap[data-open="true"] .panel {
          display: block;
        }
        .head {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 8px;
          padding: 9px 10px;
          border-bottom: 1px solid rgba(150, 165, 185, .24);
          background: rgba(31, 38, 49, .96);
        }
        .title {
          font-weight: 700;
        }
        .hide {
          border: 1px solid rgba(150, 165, 185, .36);
          border-radius: 6px;
          padding: 3px 7px;
          background: rgba(13, 16, 21, .8);
          color: #c7d0dc;
          cursor: pointer;
        }
        .body {
          display: grid;
          grid-template-columns: 74px minmax(0, 1fr);
          gap: 6px 9px;
          padding: 10px;
        }
        .key { color: #98a4b3; }
        .value {
          min-width: 0;
          overflow: hidden;
          color: #dce6f1;
          text-overflow: ellipsis;
          white-space: nowrap;
        }
      </style>
      <div class="wrap" data-state="injected" data-open="false">
        <button class="pill" type="button" title="TMWD debug badge">
          <span class="dot"></span>
          <span>TMWD</span>
        </button>
        <section class="panel">
          <div class="head">
            <span class="title">TMWD Debug</span>
            <button class="hide" type="button">Hide</button>
          </div>
          <div class="body">
            <div class="key">Content</div><div class="value" data-field="content">Injected</div>
            <div class="key">Bridge</div><div class="value" data-field="bridge">Checking</div>
            <div class="key">Tab</div><div class="value" data-field="tab">-</div>
            <div class="key">URL</div><div class="value" data-field="url"></div>
            <div class="key">Checked</div><div class="value" data-field="checked">-</div>
          </div>
        </section>
      </div>
    `;
    const root = badgeShadow.querySelector('.wrap');
    badgeShadow.querySelector('[data-field="url"]').textContent = location.href;
    badgeShadow.querySelector('.pill').addEventListener('click', () => {
      root.dataset.open = root.dataset.open === 'true' ? 'false' : 'true';
      refreshBadgeState();
    });
    badgeShadow.querySelector('.hide').addEventListener('click', async () => {
      await chrome.storage.local.set({ [BADGE_STORAGE_KEY]: false });
      removeBadge();
    });
    (document.body || document.documentElement).appendChild(badgeHost);
  }

  function removeBadge() {
    if (badgeTimer) {
      clearTimeout(badgeTimer);
      badgeTimer = null;
    }
    badgeHost?.remove();
    badgeHost = null;
    badgeShadow = null;
  }

  async function refreshBadgeState() {
    if (!badgeShadow) return;
    if (badgeTimer) {
      clearTimeout(badgeTimer);
      badgeTimer = null;
    }
    const root = badgeShadow.querySelector('.wrap');
    const bridge = badgeShadow.querySelector('[data-field="bridge"]');
    const tab = badgeShadow.querySelector('[data-field="tab"]');
    const checked = badgeShadow.querySelector('[data-field="checked"]');
    try {
      const resp = await chrome.runtime.sendMessage({ cmd: 'bridgeHealth' });
      if (resp?.ok && resp.data?.extension_connected) {
        root.dataset.state = 'connected';
        bridge.textContent = 'Bridge connected';
      } else if (resp?.ok) {
        root.dataset.state = 'injected';
        bridge.textContent = 'Bridge ready';
      } else {
        root.dataset.state = 'offline';
        bridge.textContent = 'Bridge offline';
      }
      tab.textContent = resp?.data?.sender_tab?.id ? String(resp.data.sender_tab.id) : '-';
    } catch (_) {
      root.dataset.state = 'offline';
      bridge.textContent = 'Bridge offline';
      tab.textContent = '-';
    }
    checked.textContent = new Date().toLocaleTimeString();
    badgeTimer = setTimeout(refreshBadgeState, 5000);
  }
})();
