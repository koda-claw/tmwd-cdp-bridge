#!/usr/bin/env node
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");

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

function binPath() {
  if (process.env.TMWD_BIN) return process.env.TMWD_BIN;
  const exe = process.platform === "win32" ? "tmwd-cdp-bridge.exe" : "tmwd-cdp-bridge";
  return path.join(root, "target", "debug", exe);
}

async function waitForJson(url, timeoutMs = 10000) {
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
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw lastError || new Error(`timeout waiting for ${url}`);
}

async function rpc(httpPort, token, body, auth = true) {
  const headers = { "Content-Type": "application/json" };
  if (auth) headers.Authorization = `Bearer ${token}`;
  const res = await fetch(`http://127.0.0.1:${httpPort}/v1/rpc`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  return { status: res.status, body: await res.json() };
}

async function waitUntilDown(url, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await fetch(url);
    } catch {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new Error(`${url} stayed up`);
}

async function main() {
  const appDir = await mkdtemp(path.join(tmpdir(), "tmwd-no-extension-"));
  const wsPort = await freePort();
  const httpPort = await freePort();
  const bridgeBin = binPath();
  let bridge;

  try {
    await writeFile(path.join(appDir, "version"), "2.0");
    const env = {
      ...process.env,
      CDP_BRIDGE_APP_DIR: appDir,
      CDP_BRIDGE_WS_PORT: String(wsPort),
      CDP_BRIDGE_HTTP_PORT: String(httpPort),
    };
    bridge = spawn(bridgeBin, ["start"], {
      cwd: root,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    bridge.stdout.on("data", (d) => process.stdout.write(`[bridge] ${d}`));
    bridge.stderr.on("data", (d) => process.stderr.write(`[bridge] ${d}`));

    const health = await waitForJson(`http://127.0.0.1:${httpPort}/health`);
    if (health.server !== "tmwd-cdp-bridge") throw new Error(`bad health: ${JSON.stringify(health)}`);
    if (health.extension_connected !== false) throw new Error(`extension unexpectedly connected: ${JSON.stringify(health)}`);

    const token = (await readFile(path.join(appDir, "token"), "utf8")).trim();
    const unauth = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: "unauth" }, false);
    if (unauth.status !== 401 || unauth.body.r?.error?.code !== "UNAUTHORIZED") {
      throw new Error(`unexpected unauth response: ${JSON.stringify(unauth)}`);
    }

    const sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: "sessions" });
    if (sessions.status !== 200 || !Array.isArray(sessions.body.r?.data)) {
      throw new Error(`unexpected sessions response: ${JSON.stringify(sessions)}`);
    }

    const noExtension = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "no-extension",
      code: "document.title",
    });
    if (noExtension.body.r?.error?.code !== "NO_EXTENSION") {
      throw new Error(`unexpected no-extension response: ${JSON.stringify(noExtension)}`);
    }

    const status = await new Promise((resolve, reject) => {
      const child = spawn(bridgeBin, ["status"], { cwd: root, env });
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (d) => { stdout += d; });
      child.stderr.on("data", (d) => { stderr += d; });
      child.on("error", reject);
      child.on("close", (code) => {
        if (code !== 0) reject(new Error(`status failed ${code}: ${stderr}`));
        else resolve(JSON.parse(stdout));
      });
    });
    if (status.server?.owned_by_tmwd !== true || status.pid_file?.present !== true) {
      throw new Error(`unexpected status: ${JSON.stringify(status)}`);
    }

    const stopped = await new Promise((resolve, reject) => {
      const child = spawn(bridgeBin, ["stop"], { cwd: root, env });
      let stderr = "";
      child.stderr.on("data", (d) => { stderr += d; });
      child.on("error", reject);
      child.on("close", (code) => {
        if (code !== 0) reject(new Error(`stop failed ${code}: ${stderr}`));
        else resolve(true);
      });
    });
    if (!stopped) throw new Error("stop did not complete");
    await waitUntilDown(`http://127.0.0.1:${httpPort}/health`);
    bridge = null;

    console.log(JSON.stringify({
      status: "ok",
      ws_port: wsPort,
      http_port: httpPort,
      health: {
        server: health.server,
        extension_connected: health.extension_connected,
      },
      rpc: {
        unauthorized: unauth.body.r.error.code,
        sessions: sessions.body.r.data.length,
        no_extension: noExtension.body.r.error.code,
      },
      stopped: true,
    }, null, 2));
  } finally {
    if (bridge) {
      bridge.kill("SIGKILL");
      await new Promise((resolve) => bridge.once("close", resolve));
    }
    await rm(appDir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err?.stack || err?.message || String(err));
  process.exit(1);
});
