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

function bridgeBin() {
  if (process.env.TMWD_BIN) return process.env.TMWD_BIN;
  return path.join(root, "target", "debug", process.platform === "win32" ? "tmwd-cdp-bridge.exe" : "tmwd-cdp-bridge");
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

async function rpc(httpPort, token, payload) {
  const res = await fetch(`http://127.0.0.1:${httpPort}/v1/rpc`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  return { status: res.status, body: await res.json() };
}

function runCli(args, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(bridgeBin(), args, { cwd: root, env });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => { stdout += d; });
    child.stderr.on("data", (d) => { stderr += d; });
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

async function main() {
  const appDir = await mkdtemp(path.join(tmpdir(), "tmwd-skill-flow-"));
  const wsPort = await freePort();
  const httpPort = await freePort();
  let bridge = null;
  try {
    const manifest = JSON.parse(await readFile(path.join(root, "extension", "manifest.json"), "utf8"));
    await writeFile(path.join(appDir, "version"), manifest.version);
    const env = {
      ...process.env,
      CDP_BRIDGE_APP_DIR: appDir,
      CDP_BRIDGE_WS_PORT: String(wsPort),
      CDP_BRIDGE_HTTP_PORT: String(httpPort),
    };

    const initialStatus = await runCli(["status", "--json"], env);
    if (initialStatus.code !== 0) throw new Error(`status failed: ${initialStatus.stderr}`);
    const initial = JSON.parse(initialStatus.stdout);
    if (initial.server?.running !== false) throw new Error(`expected no running server: ${initialStatus.stdout}`);

    bridge = spawn(bridgeBin(), ["start"], { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
    bridge.stdout.on("data", (d) => process.stdout.write(`[bridge] ${d}`));
    bridge.stderr.on("data", (d) => process.stderr.write(`[bridge] ${d}`));

    const health = await waitForJson(`http://127.0.0.1:${httpPort}/health`);
    if (health.server !== "tmwd-cdp-bridge") throw new Error(`bad health: ${JSON.stringify(health)}`);
    if (health.extension_id !== "eghifjkffmcmffejmaaeicejpfopplem") throw new Error(`bad extension id: ${JSON.stringify(health)}`);

    const token = (await readFile(path.join(appDir, "token"), "utf8")).trim();
    if (!token) throw new Error("empty token");

    const sessions = await rpc(httpPort, token, { cmd: "get_all_sessions", request_id: "skill-sessions" });
    if (sessions.status !== 200 || !Array.isArray(sessions.body.r?.data)) {
      throw new Error(`bad sessions: ${JSON.stringify(sessions)}`);
    }

    const noExtension = await rpc(httpPort, token, {
      cmd: "execute_js",
      request_id: "skill-no-extension",
      code: "document.title",
    });
    if (noExtension.body.r?.error?.code !== "NO_EXTENSION") {
      throw new Error(`expected NO_EXTENSION: ${JSON.stringify(noExtension)}`);
    }

    const stop = await runCli(["stop"], env);
    if (stop.code !== 0) throw new Error(`stop failed: ${stop.stderr}`);
    bridge = null;

    console.log(JSON.stringify({
      status: "ok",
      capability_surface: ["SKILL.md", "shell", "file-read", "curl-equivalent HTTP"],
      health: {
        server: health.server,
        extension_connected: health.extension_connected,
        extension_id: health.extension_id,
      },
      rpc: {
        sessions_request_id: sessions.body.r.request_id,
        no_extension_code: noExtension.body.r.error.code,
      },
      stopped_owned_bridge: true,
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
