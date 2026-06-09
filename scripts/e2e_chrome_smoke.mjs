#!/usr/bin/env node
import { spawn } from "node:child_process";
import { cp, mkdtemp, rm, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import net from "node:net";
import http from "node:http";

const root = path.resolve(import.meta.dirname, "..");
const browserBin =
  process.env.BROWSER_BIN ||
  process.env.CHROME_BIN ||
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
    server.on("error", reject);
  });
}

async function waitForJson(url, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return await res.json();
      lastError = new Error(`${url} returned ${res.status}`);
    } catch (err) {
      lastError = err;
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw lastError || new Error(`timeout waiting for ${url}`);
}

async function openPage(debugPort, url) {
  const res = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent(url)}`, {
    method: "PUT",
  });
  if (!res.ok) throw new Error(`open page failed: ${res.status} ${await res.text()}`);
  return await res.json();
}

async function main() {
  const appDir = await mkdtemp(path.join(tmpdir(), "tmwd-app-"));
  const profileDir = await mkdtemp(path.join(tmpdir(), "tmwd-chrome-"));
  const extensionDir = await mkdtemp(path.join(tmpdir(), "tmwd-extension-"));
  const wsPort = await freePort();
  const httpPort = await freePort();
  const debugPort = await freePort();
  const pagePort = await freePort();
  let bridge;
  let chrome;
  let pageServer;

  try {
    pageServer = http.createServer((_, res) => {
      res.writeHead(200, { "Content-Type": "text/html" });
      res.end("<!doctype html><title>TMWD Smoke</title><h1 id=\"x\">Hello</h1>");
    });
    await new Promise((resolve) => pageServer.listen(pagePort, "127.0.0.1", resolve));

    await cp(path.join(root, "extension"), extensionDir, { recursive: true });
    const manifest = JSON.parse(await readFile(path.join(extensionDir, "manifest.json"), "utf8"));
    await writeFile(path.join(appDir, "version"), manifest.version);
    const backgroundPath = path.join(extensionDir, "background.js");
    const background = await readFile(backgroundPath, "utf8");
    await writeFile(
      backgroundPath,
      background
        .replace("const DEFAULT_WS_URL = 'ws://127.0.0.1:18765';", `const DEFAULT_WS_URL = 'ws://127.0.0.1:${wsPort}';`)
        .replace("const DEFAULT_HEALTH_URL = 'http://127.0.0.1:18766/health';", `const DEFAULT_HEALTH_URL = 'http://127.0.0.1:${httpPort}/health';`),
    );

    chrome = spawn(browserBin, [
      `--user-data-dir=${profileDir}`,
      `--remote-debugging-port=${debugPort}`,
      `--load-extension=${extensionDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-features=DialMediaRouteProvider",
      "about:blank",
    ], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    chrome.stdout.on("data", (d) => process.stdout.write(`[chrome] ${d}`));
    chrome.stderr.on("data", (d) => process.stderr.write(`[chrome] ${d}`));

    await waitForJson(`http://127.0.0.1:${debugPort}/json/version`, 15000);

    const discovered = await discoverTmwdExtension(debugPort);
    const extensionId = discovered.extensionId;
    await openPage(debugPort, `chrome-extension://${extensionId}/popup.html`);
    const allowedOrigin = `chrome-extension://${extensionId}`;

    bridge = spawn("cargo", ["run", "--", "start"], {
      cwd: root,
      env: {
        ...process.env,
        CDP_BRIDGE_APP_DIR: appDir,
        CDP_BRIDGE_WS_PORT: String(wsPort),
        CDP_BRIDGE_HTTP_PORT: String(httpPort),
        CDP_BRIDGE_ALLOWED_EXTENSION_ORIGIN: allowedOrigin,
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    bridge.stdout.on("data", (d) => process.stdout.write(`[bridge] ${d}`));
    bridge.stderr.on("data", (d) => process.stderr.write(`[bridge] ${d}`));
    const initialHealth = await waitForJson(`http://127.0.0.1:${httpPort}/health`);
    console.log("[e2e] bridge health", JSON.stringify(initialHealth));

    const extTarget = await waitForExtensionTarget(debugPort, extensionId);
    const extState = await evalInPage(extTarget.webSocketDebuggerUrl, `
      ({
        runtimeId: chrome.runtime.id,
        manifest: chrome.runtime.getManifest(),
        hasTabs: !!chrome.tabs,
        hasScripting: !!chrome.scripting
      })
    `);
    console.log("[e2e] extension target state", JSON.stringify(extState));
    const token = (await readFile(path.join(appDir, "token"), "utf8")).trim();
    await evalInPage(extTarget.webSocketDebuggerUrl, injectedClientCode(wsPort, token));
    await openPage(debugPort, `chrome-extension://${extensionId}/popup.html`).catch(() => {});
    await waitForExtension(httpPort);

    const pageUrl = `http://127.0.0.1:${pagePort}/`;
    const createdTab = await evalInPage(extTarget.webSocketDebuggerUrl, `
      new Promise((resolve, reject) => {
        chrome.tabs.create({ url: ${JSON.stringify(pageUrl)}, active: true }, (tab) => {
          if (chrome.runtime.lastError) reject(chrome.runtime.lastError);
          else resolve({ id: tab.id, url: tab.url, title: tab.title, active: tab.active });
        });
      })
    `);
    console.log("[e2e] created tab", JSON.stringify(createdTab));
    await new Promise((r) => setTimeout(r, 1000));
    const tabsSent = await evalInPage(extTarget.webSocketDebuggerUrl, `
      (async () => {
        const ws = globalThis.__tmwdTestWs;
        if (!ws || ws.readyState !== WebSocket.OPEN) return { sent: false, readyState: ws?.readyState };
        ws.send(JSON.stringify({
          type: "tabs_update",
          tabs: [{ id: ${createdTab.id}, url: ${JSON.stringify(pageUrl)}, title: "TMWD Smoke", active: true, window_id: 1 }]
        }));
        return { sent: true, count: 1, tabs: [${JSON.stringify(pageUrl)}] };
      })()
    `);
    console.log("[e2e] manual tabs_update", JSON.stringify(tabsSent));
    await new Promise((r) => setTimeout(r, 500));

    const sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: "sessions" });
    if (!Array.isArray(sessions.r.data) || sessions.r.data.length === 0) {
      throw new Error(`no sessions registered: ${JSON.stringify(sessions)}`);
    }
    const target = sessions.r.data.find((s) => String(s.url || "") === pageUrl);
    if (!target) throw new Error(`test page session not found: ${JSON.stringify(sessions)}`);

    const exec = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "exec",
      sessionId: String(target.id),
      code: "document.title + ':' + document.querySelector('#x').textContent",
      timeout: 10,
    });
    if (exec.r.data !== "TMWD Smoke:Hello") {
      throw new Error(`unexpected execute result: ${JSON.stringify(exec)}`);
    }

    console.log(JSON.stringify({
      status: "ok",
      extensionId,
      sessions: sessions.r.data.length,
      target: { id: target.id, title: target.title, url: target.url },
      exec: exec.r.data,
    }, null, 2));
  } finally {
    if (chrome) chrome.kill("SIGKILL");
    if (bridge) bridge.kill("SIGKILL");
    if (pageServer) await new Promise((resolve) => pageServer.close(resolve));
    await rm(appDir, { recursive: true, force: true });
    await rm(profileDir, { recursive: true, force: true });
    await rm(extensionDir, { recursive: true, force: true });
  }
}

function injectedClientCode(wsPort, token) {
  return `
    (() => {
      const token = ${JSON.stringify(token)};
      const ws = new WebSocket("ws://127.0.0.1:${wsPort}");
      globalThis.__tmwdTestWs = ws;
      async function execute(data) {
        try {
          const result = await chrome.scripting.executeScript({
            target: { tabId: Number(data.tabId) },
            world: "MAIN",
            func: async (code) => await eval(code),
            args: [String(data.code)]
          });
          ws.send(JSON.stringify({ type: "result", id: data.id, result: result?.[0]?.result, newTabs: [] }));
        } catch (error) {
          ws.send(JSON.stringify({ type: "error", id: data.id, error: { message: error.message || String(error) }, newTabs: [] }));
        }
      }
      ws.onmessage = async (event) => {
        const data = JSON.parse(event.data);
        if (data.type === "auth_required") ws.send(JSON.stringify({ type: "auth", token }));
        else if (data.type === "auth_ok") ws.send(JSON.stringify({ type: "ext_ready", tabs: [] }));
        else if (data.id && data.code) await execute(data);
      };
      return true;
    })();
  `;
}

async function waitForExtensionTarget(debugPort, extensionId) {
  const deadline = Date.now() + 10000;
  let lastTargets = [];
  while (Date.now() < deadline) {
    const targets = await fetch(`http://127.0.0.1:${debugPort}/json/list`).then((r) => r.json());
    lastTargets = targets
      .filter((t) => String(t.url || "").includes("chrome-extension://"))
      .map((t) => ({ type: t.type, title: t.title, url: t.url, hasWs: !!t.webSocketDebuggerUrl }));
    const target = targets.find((t) =>
      String(t.url || "").startsWith(`chrome-extension://${extensionId}/`) &&
      t.webSocketDebuggerUrl &&
      (t.type === "page" || t.type === "service_worker" || t.type === "background_page")
    );
    if (target) return target;
    await openPage(debugPort, `chrome-extension://${extensionId}/popup.html`).catch(() => {});
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`extension target did not appear for ${extensionId}; targets=${JSON.stringify(lastTargets)}`);
}

async function discoverTmwdExtension(debugPort) {
  const deadline = Date.now() + 10000;
  let last = [];
  while (Date.now() < deadline) {
    const targets = await fetch(`http://127.0.0.1:${debugPort}/json/list`).then((r) => r.json());
    const candidates = targets.filter((t) =>
      String(t.url || "").startsWith("chrome-extension://") && t.webSocketDebuggerUrl
    );
    last = candidates.map((t) => ({ type: t.type, title: t.title, url: t.url }));
    for (const target of candidates) {
      const extensionId = String(target.url).match(/^chrome-extension:\/\/([a-p]{32})\//)?.[1];
      if (target.type === "service_worker" && extensionId) {
        return { extensionId, target, manifest: null };
      }
      try {
        const manifest = await evalInPage(target.webSocketDebuggerUrl, `
          chrome.runtime?.getManifest ? chrome.runtime.getManifest() : null
        `);
        if (manifest?.name === "TMWD CDP Bridge") {
          if (extensionId) return { extensionId, target, manifest };
        }
      } catch (_) {}
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`could not discover TMWD extension; targets=${JSON.stringify(last)}`);
}

async function evalInPage(webSocketDebuggerUrl, expression) {
  const ws = new WebSocket(webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  let id = 0;
  function call(method, params = {}) {
    const msgId = ++id;
    ws.send(JSON.stringify({ id: msgId, method, params }));
    return new Promise((resolve, reject) => {
      const onMessage = (event) => {
        const msg = JSON.parse(event.data);
        if (msg.id !== msgId) return;
        ws.removeEventListener("message", onMessage);
        if (msg.error) reject(new Error(JSON.stringify(msg.error)));
        else resolve(msg.result || {});
      };
      ws.addEventListener("message", onMessage);
    });
  }
  try {
    const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result?.value;
  } finally {
    ws.close();
  }
}

async function rpc(httpPort, token, body) {
  const res = await fetch(`http://127.0.0.1:${httpPort}/v1/rpc`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`rpc failed ${res.status}: ${await res.text()}`);
  return await res.json();
}

async function waitForExtension(httpPort) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    const h = await waitForJson(`http://127.0.0.1:${httpPort}/health`, 1000);
    if (h.extension_connected) return;
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error("extension did not connect");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
