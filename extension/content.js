;(function(){
  if (/streamlit/i.test(document.title)) return;

  const requestIds = Array.isArray(globalThis.TMWD_REQUEST_IDS)
    ? globalThis.TMWD_REQUEST_IDS
    : ['__tmwd_cdp_bridge_request', '__ljq_045ef1'];
  const selector = requestIds.map(id => `#${cssEscape(id)}`).join(',');

  document.querySelectorAll('meta[http-equiv="Content-Security-Policy"]').forEach(e => e.remove());

  new MutationObserver(muts => {
    for (const m of muts) {
      for (const n of m.addedNodes) {
        for (const el of findRequestElements(n)) handle(el);
      }
    }
  }).observe(document.documentElement, { childList: true, subtree: true });
  document.querySelectorAll(selector).forEach(handle);

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
})();
