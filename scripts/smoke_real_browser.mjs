#!/usr/bin/env node
import { spawn } from "node:child_process";
import { cp, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const FIXED_EXTENSION_ID = "eghifjkffmcmffejmaaeicejpfopplem";
const DEFAULT_BROWSER =
  process.env.BROWSER_BIN ||
  process.env.CHROME_BIN ||
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";

function usage() {
  console.error(`usage: node scripts/smoke_real_browser.mjs [options]

Options:
  --browser-bin <path>  Chrome or Edge executable (default: BROWSER_BIN/CHROME_BIN or Edge on macOS)
  --keep-profile        Do not delete the temporary browser profile after the run
  --keep-extension      Do not delete the temporary unpacked extension after the run
  --keep-app            Do not delete the temporary bridge app dir after the run
  --debug-on-failure    Print browser/bridge paths, health, and DevTools target diagnostics on failure
  --help                Show this help
`);
}

function parseArgs(argv) {
  const args = {
    browserBin: DEFAULT_BROWSER,
    keepProfile: false,
    keepExtension: false,
    keepApp: false,
    debugOnFailure: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help") {
      usage();
      process.exit(0);
    }
    if (arg === "--keep-profile") args.keepProfile = true;
    else if (arg === "--keep-extension") args.keepExtension = true;
    else if (arg === "--keep-app") args.keepApp = true;
    else if (arg === "--debug-on-failure") args.debugOnFailure = true;
    else if (arg === "--browser-bin") {
      i += 1;
      if (i >= argv.length) throw new Error("--browser-bin requires a value");
      args.browserBin = argv[i];
    } else {
      throw new Error(`unknown option: ${arg}`);
    }
  }
  return args;
}

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

async function tryFetchJson(url) {
  try {
    const res = await fetch(url);
    const text = await res.text();
    let json = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch (_) {
      json = { raw: text };
    }
    return { ok: res.ok, status: res.status, json };
  } catch (err) {
    return { ok: false, error: err?.message || String(err) };
  }
}

async function debugTargets(debugPort) {
  const listed = await tryFetchJson(`http://127.0.0.1:${debugPort}/json/list`);
  if (!listed.ok || !Array.isArray(listed.json)) return listed;
  const targets = listed.json.map((t) => ({
    id: t.id,
    type: t.type,
    title: t.title,
    url: t.url,
    has_websocket: Boolean(t.webSocketDebuggerUrl),
  }));
  const extensionTargets = targets.filter((t) => String(t.url || "").startsWith("chrome-extension://"));
  const fixedIdTargets = targets.filter((t) => String(t.url || "").startsWith(`chrome-extension://${FIXED_EXTENSION_ID}/`));
  return {
    ok: true,
    status: listed.status,
    target_count: targets.length,
    extension_target_count: extensionTargets.length,
    fixed_id_target_count: fixedIdTargets.length,
    extension_targets: extensionTargets,
    all_targets: targets,
  };
}

async function evalInTarget(webSocketDebuggerUrl, expression) {
  const ws = new WebSocket(webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  let id = 0;
  async function call(method, params = {}) {
    const msgId = ++id;
    ws.send(JSON.stringify({ id: msgId, method, params }));
    return await new Promise((resolve, reject) => {
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
    if (result.exceptionDetails) {
      return { ok: false, exception: result.exceptionDetails };
    }
    return { ok: true, value: result.result?.value };
  } catch (err) {
    return { ok: false, error: err?.message || String(err) };
  } finally {
    ws.close();
  }
}

async function debugExtensionRuntime(debugPort) {
  const listed = await tryFetchJson(`http://127.0.0.1:${debugPort}/json/list`);
  if (!listed.ok || !Array.isArray(listed.json)) return listed;
  const fixedTargets = listed.json.filter((t) =>
    String(t.url || "").startsWith(`chrome-extension://${FIXED_EXTENSION_ID}/`) &&
    t.webSocketDebuggerUrl
  );
  const target = fixedTargets.find((t) => t.type === "service_worker") ||
    fixedTargets.find((t) => t.type === "background_page") ||
    fixedTargets.find((t) => t.type === "page");
  if (!target) {
    return {
      ok: false,
      error: `no DevTools target for fixed extension id ${FIXED_EXTENSION_ID}`,
      fixed_target_count: fixedTargets.length,
    };
  }
  const manifest = await evalInTarget(target.webSocketDebuggerUrl, "chrome.runtime.getManifest()");
  const sendMessage = await evalInTarget(target.webSocketDebuggerUrl, `
    new Promise((resolve) => {
      chrome.runtime.sendMessage({ cmd: 'tabs' }, (resp) => {
        resolve({ resp: resp || null, lastError: chrome.runtime.lastError?.message || null });
      });
    })
  `);
  return {
    ok: true,
    target: {
      type: target.type,
      title: target.title,
      url: target.url,
    },
    manifest,
    send_message_tabs: sendMessage,
  };
}

async function printFailureDiagnostics(args, paths) {
  if (!args.debugOnFailure) return;
  const health = await tryFetchJson(`http://127.0.0.1:${paths.httpPort}/health`);
  const version = await tryFetchJson(`http://127.0.0.1:${paths.debugPort}/json/version`);
  const targets = await debugTargets(paths.debugPort);
  const extensionRuntime = await debugExtensionRuntime(paths.debugPort);
  console.error("[smoke] failure diagnostics:");
  console.error(JSON.stringify({
    browser: args.browserBin,
    fixed_extension_id: FIXED_EXTENSION_ID,
    app_dir: paths.appDir,
    profile_dir: paths.profileDir,
    extension_dir: paths.extensionDir,
    ws_port: paths.wsPort,
    http_port: paths.httpPort,
    debug_port: paths.debugPort,
    health,
    devtools_version: version,
    devtools_targets: targets,
    extension_runtime: extensionRuntime,
  }, null, 2));
}

async function waitForJson(url, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return await res.json();
      lastError = new Error(`${url} returned ${res.status}: ${await res.text()}`);
    } catch (err) {
      lastError = err;
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw lastError || new Error(`timeout waiting for ${url}`);
}

async function openPage(debugPort, url) {
  const res = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent(url)}`, {
    method: "PUT",
  });
  if (!res.ok) throw new Error(`open page failed: ${res.status}: ${await res.text()}`);
  return await res.json();
}

async function closeTarget(debugPort, targetId) {
  const res = await fetch(`http://127.0.0.1:${debugPort}/json/close/${targetId}`);
  if (!res.ok) throw new Error(`close target failed: ${res.status}: ${await res.text()}`);
}

async function rpc(httpPort, token, body) {
  const json = await rpcRaw(httpPort, token, body);
  if (json.r?.error) throw new Error(`rpc ${body.request_id || body.cmd} failed: ${JSON.stringify(json)}`);
  return json;
}

async function rpcAllowError(httpPort, token, body) {
  return await rpcRaw(httpPort, token, body);
}

async function rpcRaw(httpPort, token, body) {
  const res = await fetch(`http://127.0.0.1:${httpPort}/v1/rpc`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const json = await res.json();
  if (!res.ok) throw new Error(`rpc HTTP ${res.status}: ${JSON.stringify(json)}`);
  return json;
}

async function waitForExtension(httpPort, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await waitForJson(`http://127.0.0.1:${httpPort}/health`, 2000);
    if (last.extension_connected) return last;
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`extension did not connect: ${JSON.stringify(last)}`);
}

async function waitForSession(httpPort, token, predicate, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let sessions;
  while (Date.now() < deadline) {
    sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: `sessions-${Date.now()}` });
    const found = (sessions.r.data || []).find(predicate);
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`session not found: ${JSON.stringify(sessions)}`);
}

async function waitForNoSession(httpPort, token, tabId, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  let sessions;
  while (Date.now() < deadline) {
    sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: `sessions-${Date.now()}` });
    if (!(sessions.r.data || []).some((tab) => tab.id === tabId)) return true;
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`closed tab still present: ${JSON.stringify(sessions)}`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const appDir = await mkdtemp(path.join(tmpdir(), "tmwd-app-"));
  const profileDir = await mkdtemp(path.join(tmpdir(), "tmwd-browser-"));
  const extensionDir = await mkdtemp(path.join(tmpdir(), "tmwd-extension-"));
  const wsPort = await freePort();
  const httpPort = await freePort();
  const debugPort = await freePort();
  const pagePort = await freePort();
  let browser;
  let bridge;
  let pageServer;
  let failed = false;

  try {
    pageServer = http.createServer((req, res) => {
      if (req.url === "/csp") {
        res.writeHead(200, {
          "Content-Type": "text/html",
          "Content-Security-Policy": "default-src 'self'; script-src 'self'",
        });
        res.end("<!doctype html><title>TMWD CSP</title><h1 id=\"x\">CSP</h1>");
        return;
      }
      res.writeHead(200, { "Content-Type": "text/html" });
      res.end("<!doctype html><title>TMWD Normal</title><h1 id=\"x\">Normal</h1>");
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

    browser = spawn(args.browserBin, [
      `--user-data-dir=${profileDir}`,
      `--remote-debugging-port=${debugPort}`,
      `--disable-extensions-except=${extensionDir}`,
      `--load-extension=${extensionDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-features=DialMediaRouteProvider",
      "about:blank",
    ], { stdio: ["ignore", "pipe", "pipe"] });
    browser.stdout.on("data", (d) => process.stdout.write(`[browser] ${d}`));
    browser.stderr.on("data", (d) => process.stderr.write(`[browser] ${d}`));
    await waitForJson(`http://127.0.0.1:${debugPort}/json/version`);

    bridge = spawn("cargo", ["run", "--", "start"], {
      cwd: root,
      env: {
        ...process.env,
        CDP_BRIDGE_APP_DIR: appDir,
        CDP_BRIDGE_WS_PORT: String(wsPort),
        CDP_BRIDGE_HTTP_PORT: String(httpPort),
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    bridge.stdout.on("data", (d) => process.stdout.write(`[bridge] ${d}`));
    bridge.stderr.on("data", (d) => process.stderr.write(`[bridge] ${d}`));
    await waitForJson(`http://127.0.0.1:${httpPort}/health`);
    await openPage(debugPort, `chrome-extension://${FIXED_EXTENSION_ID}/popup.html`).catch(() => {});
    const health = await waitForExtension(httpPort);
    const token = (await readFile(path.join(appDir, "token"), "utf8")).trim();

    const normalUrl = `http://127.0.0.1:${pagePort}/`;
    const normalTarget = await openPage(debugPort, normalUrl);
    const normalSession = await waitForSession(httpPort, token, (tab) => tab.url === normalUrl);
    const normalExec = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "normal-exec",
      sessionId: String(normalSession.id),
      code: "document.title + ':' + document.querySelector('#x').textContent",
      timeout: 15,
    });
    if (normalExec.r.data !== "TMWD Normal:Normal") {
      throw new Error(`unexpected normal result: ${JSON.stringify(normalExec)}`);
    }

    const cdpDirect = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "cdp-direct",
      sessionId: String(normalSession.id),
      mode: "cdp",
      code: {
        method: "Runtime.evaluate",
        params: {
          expression: "document.title",
          awaitPromise: true,
          returnByValue: true,
        },
      },
      timeout: 15,
    });
    if (cdpDirect.r.data?.result?.value !== "TMWD Normal") {
      throw new Error(`unexpected CDP direct result: ${JSON.stringify(cdpDirect)}`);
    }

    const cspUrl = `http://127.0.0.1:${pagePort}/csp`;
    const cspTarget = await openPage(debugPort, cspUrl);
    const cspSession = await waitForSession(httpPort, token, (tab) => tab.url === cspUrl);
    const fallback = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "fallback-requested",
      sessionId: String(cspSession.id),
      fallback: "cdp",
      code: "document.title + ':' + document.querySelector('#x').textContent",
      timeout: 15,
    });
    if (fallback.r.data !== "TMWD CSP:CSP") {
      throw new Error(`unexpected fallback result: ${JSON.stringify(fallback)}`);
    }

    const batch = await rpc(httpPort, token, {
      cmd: "batch",
      request_id: "batch-read",
      items: [
        {
          cmd: "execute_js",
          request_id: "batch-title",
          sessionId: String(cspSession.id),
          code: "document.title",
          timeout: 15,
        },
        {
          cmd: "execute_js",
          request_id: "batch-url",
          sessionId: String(cspSession.id),
          code: "location.href",
          timeout: 15,
        },
      ],
    });
    const batchItems = batch.r.items || [];
    if (batchItems[0]?.data !== "TMWD CSP" || batchItems[1]?.data !== cspUrl) {
      throw new Error(`unexpected batch result: ${JSON.stringify(batch)}`);
    }

    const expectedError = await rpcAllowError(httpPort, token, {
      cmd: "execute_js",
      request_id: "expected-error",
      sessionId: String(cspSession.id),
      code: "(() => { throw new Error('expected smoke failure') })()",
      timeout: 15,
    });
    if (expectedError.r?.error?.code !== "EXEC_ERROR") {
      throw new Error(`expected EXEC_ERROR: ${JSON.stringify(expectedError)}`);
    }
    const recovery = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "after-error-recovery",
      sessionId: String(cspSession.id),
      code: "document.querySelector('#x').textContent",
      timeout: 15,
    });
    if (recovery.r.data !== "CSP") {
      throw new Error(`unexpected recovery result: ${JSON.stringify(recovery)}`);
    }

    await closeTarget(debugPort, normalTarget.id);
    await waitForNoSession(httpPort, token, normalSession.id);

    const reload = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "extension-reload",
      sessionId: String(cspSession.id),
      code: { cmd: "management", method: "reload" },
      timeout: 15,
    });
    if (reload.r.data?.reconnectExpected !== true) {
      throw new Error(`unexpected reload response: ${JSON.stringify(reload)}`);
    }
    const reconnected = await waitForExtension(httpPort, 30000);
    const afterReconnectSession = await waitForSession(httpPort, token, (tab) => tab.url === cspUrl, 20000);

    await closeTarget(debugPort, cspTarget.id).catch(() => {});

    console.log(JSON.stringify({
      status: "ok",
      browser: args.browserBin,
      extension_id: FIXED_EXTENSION_ID,
      health,
      normal: {
        session_id: normalSession.id,
        execute_js: normalExec.r.data,
      },
      cdp_direct: cdpDirect.r.data,
      fallback_requested: {
        session_id: cspSession.id,
        execute_js: fallback.r.data,
      },
      batch_read: {
        title: batchItems[0].data,
        url: batchItems[1].data,
      },
      error_recovery: {
        expected_error_code: expectedError.r.error.code,
        after_error: recovery.r.data,
      },
      tab_close: {
        closed_session_id: normalSession.id,
        removed_from_sessions: true,
      },
      extension_reconnect: {
        reloaded: true,
        extension_connected: reconnected.extension_connected,
        session_id_after_reconnect: afterReconnectSession.id,
      },
    }, null, 2));
  } catch (err) {
    failed = true;
    await printFailureDiagnostics(args, {
      appDir,
      profileDir,
      extensionDir,
      wsPort,
      httpPort,
      debugPort,
    });
    throw err;
  } finally {
    if (browser) browser.kill("SIGKILL");
    if (bridge) bridge.kill("SIGKILL");
    if (pageServer) await new Promise((resolve) => pageServer.close(resolve));
    if (!args.keepApp && !(failed && args.debugOnFailure)) await rm(appDir, { recursive: true, force: true });
    if (!args.keepProfile && !(failed && args.debugOnFailure)) await rm(profileDir, { recursive: true, force: true });
    if (!args.keepExtension && !(failed && args.debugOnFailure)) await rm(extensionDir, { recursive: true, force: true });
    if (args.keepApp || (failed && args.debugOnFailure)) console.error(`[smoke] kept app dir: ${appDir}`);
    if (args.keepProfile || (failed && args.debugOnFailure)) console.error(`[smoke] kept profile: ${profileDir}`);
    if (args.keepExtension || (failed && args.debugOnFailure)) console.error(`[smoke] kept extension: ${extensionDir}`);
  }
}

main().catch((err) => {
  console.error(err?.stack || err?.message || String(err));
  process.exit(1);
});
