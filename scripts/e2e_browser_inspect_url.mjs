#!/usr/bin/env node
import { spawn } from "node:child_process";
import { cp, mkdtemp, rm, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import net from "node:net";

const root = path.resolve(import.meta.dirname, "..");
const targetUrl = process.argv[2];
if (!targetUrl) {
  console.error("usage: node scripts/e2e_browser_inspect_url.mjs <url>");
  process.exit(2);
}

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

async function waitForJson(url, timeoutMs = 15000) {
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

async function rpc(httpPort, token, body) {
  const res = await fetch(`http://127.0.0.1:${httpPort}/v1/rpc`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  return await res.json();
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
  let bridge;
  let browser;

  try {
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

    browser = spawn(browserBin, [
      `--user-data-dir=${profileDir}`,
      `--remote-debugging-port=${debugPort}`,
      `--load-extension=${extensionDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-features=DialMediaRouteProvider",
      "about:blank",
    ], { stdio: ["ignore", "pipe", "pipe"] });
    browser.stdout.on("data", (d) => process.stdout.write(`[browser] ${d}`));
    browser.stderr.on("data", (d) => process.stderr.write(`[browser] ${d}`));

    await waitForJson(`http://127.0.0.1:${debugPort}/json/version`);
    await openPage(debugPort, targetUrl);

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

    let health;
    for (let i = 0; i < 30; i++) {
      try {
        health = await waitForJson(`http://127.0.0.1:${httpPort}/health`, 3000);
      } catch {
        await new Promise((r) => setTimeout(r, 500));
        continue;
      }
      if (health.extension_connected) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    const token = (await readFile(path.join(appDir, "token"), "utf8")).trim();
    await new Promise((r) => setTimeout(r, 4000));
    const sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: "sessions" });
    const tabs = sessions.r.data || [];
    const target = tabs.find((tab) => String(tab.url || "").startsWith(targetUrl)) ||
      tabs.find((tab) => String(tab.url || "").includes(new URL(targetUrl).hostname));
    if (!target) {
      throw new Error(`target tab not found: ${JSON.stringify({ health, sessions })}`);
    }
    const snapshot = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "snapshot",
      sessionId: String(target.id),
      code: `(() => {
        const rows = Array.from(document.querySelectorAll('table tr, [role=row]'))
          .map(tr => Array.from(tr.children).map(td => td.innerText.trim()).filter(Boolean))
          .filter(row => row.length);
        const links = Array.from(document.querySelectorAll('a,button,[role=button]'))
          .map((e, i) => ({ i, text: e.innerText.trim(), href: e.href || null, role: e.getAttribute('role') }))
          .filter(x => x.text || x.href)
          .slice(0, 100);
        return {
          title: document.title,
          url: location.href,
          text: document.body.innerText.slice(0, 20000),
          rows,
          links
        };
      })()`,
      timeout: 20,
    });
    console.log(JSON.stringify({ status: "ok", health, target, snapshot: snapshot.r.data }, null, 2));
  } finally {
    if (browser) browser.kill("SIGKILL");
    if (bridge) bridge.kill("SIGKILL");
    await rm(appDir, { recursive: true, force: true });
    await rm(profileDir, { recursive: true, force: true });
    await rm(extensionDir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
