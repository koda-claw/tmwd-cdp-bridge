#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { homedir, platform } from "node:os";
import path from "node:path";

const DEFAULT_HTTP_URL = "http://127.0.0.1:18766";

function usage() {
  console.error(`usage: node scripts/soak_bridge.mjs [options]

Options:
  --duration <time>    Total run time, for example 30s, 5m, 1h (default: 30m)
  --interval <time>    Delay between probes, for example 60s (default: 60s)
  --http-url <url>     Bridge HTTP base URL (default: ${DEFAULT_HTTP_URL})
  --token-file <path>  Bearer token file (default: platform app data path)
  --timeout <time>     Per HTTP request timeout (default: 5s)
  --help               Show this help
`);
}

function parseArgs(argv) {
  const args = {
    durationMs: parseDuration("30m"),
    intervalMs: parseDuration("60s"),
    httpUrl: process.env.CDP_BRIDGE_HTTP_URL || DEFAULT_HTTP_URL,
    tokenFile: process.env.CDP_BRIDGE_TOKEN_FILE || defaultTokenFile(),
    timeoutMs: parseDuration("5s"),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help") {
      usage();
      process.exit(0);
    }
    const next = () => {
      i += 1;
      if (i >= argv.length) throw new Error(`${arg} requires a value`);
      return argv[i];
    };
    if (arg === "--duration") args.durationMs = parseDuration(next());
    else if (arg === "--interval") args.intervalMs = parseDuration(next());
    else if (arg === "--http-url") args.httpUrl = next().replace(/\/+$/, "");
    else if (arg === "--token-file") args.tokenFile = next();
    else if (arg === "--timeout") args.timeoutMs = parseDuration(next());
    else throw new Error(`unknown option: ${arg}`);
  }
  if (args.durationMs <= 0) throw new Error("--duration must be positive");
  if (args.intervalMs <= 0) throw new Error("--interval must be positive");
  if (args.timeoutMs <= 0) throw new Error("--timeout must be positive");
  return args;
}

function parseDuration(value) {
  const match = String(value).trim().match(/^(\d+(?:\.\d+)?)(ms|s|m|h)?$/);
  if (!match) throw new Error(`invalid duration: ${value}`);
  const n = Number(match[1]);
  const unit = match[2] || "ms";
  const factors = { ms: 1, s: 1000, m: 60_000, h: 3_600_000 };
  return Math.max(1, Math.round(n * factors[unit]));
}

function defaultTokenFile() {
  const p = platform();
  if (p === "darwin") return path.join(homedir(), "Library", "Application Support", "tmwd-cdp-bridge", "token");
  if (p === "win32") {
    const base = process.env.LOCALAPPDATA || path.join(homedir(), "AppData", "Local");
    return path.join(base, "tmwd-cdp-bridge", "token");
  }
  const base = process.env.XDG_DATA_HOME || path.join(homedir(), ".local", "share");
  return path.join(base, "tmwd-cdp-bridge", "token");
}

async function fetchJson(url, options = {}, timeoutMs) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(new Error(`timeout after ${timeoutMs}ms`)), timeoutMs);
  try {
    const res = await fetch(url, { ...options, signal: ctrl.signal });
    const text = await res.text();
    let body = null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      throw new Error(`${url} returned non-JSON status ${res.status}: ${text.slice(0, 200)}`);
    }
    if (!res.ok) throw new Error(`${url} returned ${res.status}: ${JSON.stringify(body)}`);
    return body;
  } finally {
    clearTimeout(timer);
  }
}

async function probe(args, token) {
  const health = await fetchJson(`${args.httpUrl}/health`, {}, args.timeoutMs);
  if (health.server !== "tmwd-cdp-bridge") {
    throw new Error(`unexpected health server: ${JSON.stringify(health)}`);
  }
  const rpc = await fetchJson(`${args.httpUrl}/v1/rpc`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      cmd: "get_all_sessions",
      request_id: `soak-${Date.now()}`,
    }),
  }, args.timeoutMs);
  if (!rpc.r || rpc.r.error) {
    throw new Error(`rpc failed: ${JSON.stringify(rpc)}`);
  }
  return {
    extension_connected: Boolean(health.extension_connected),
    extension_last_seen_age_ms: health.extension_last_seen_age_ms ?? null,
    sessions: Array.isArray(rpc.r.data) ? rpc.r.data.length : null,
  };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const token = (await readFile(args.tokenFile, "utf8")).trim();
  if (!token) throw new Error(`empty token file: ${args.tokenFile}`);

  const startedAt = Date.now();
  const deadline = startedAt + args.durationMs;
  let passes = 0;
  let failures = 0;
  let lastError = null;
  let lastOk = null;

  while (Date.now() < deadline) {
    try {
      lastOk = await probe(args, token);
      passes += 1;
    } catch (err) {
      failures += 1;
      lastError = err?.message || String(err);
      console.error(`[soak] probe failed: ${lastError}`);
    }
    const remainingMs = deadline - Date.now();
    if (remainingMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, Math.min(args.intervalMs, remainingMs)));
    }
  }

  const summary = {
    status: failures === 0 && passes > 0 ? "ok" : "failed",
    started_at_unix_ms: startedAt,
    duration_ms: Date.now() - startedAt,
    requested_duration_ms: args.durationMs,
    interval_ms: args.intervalMs,
    http_url: args.httpUrl,
    token_file: args.tokenFile,
    passes,
    failures,
    last_ok: lastOk,
    last_error: lastError,
    cleanup: {
      owned_processes_started: 0,
      owned_processes_stopped: 0,
    },
  };
  console.log(JSON.stringify(summary, null, 2));
  if (summary.status !== "ok") process.exit(1);
}

main().catch((err) => {
  console.error(err?.stack || err?.message || String(err));
  process.exit(1);
});
