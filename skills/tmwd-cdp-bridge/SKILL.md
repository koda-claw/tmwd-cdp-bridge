---
name: tmwd-cdp-bridge
description: Use tmwd-cdp-bridge to control a local Chrome or Edge browser through the bundled MV3 extension. Use when an agent needs to inspect real browser tabs, execute JavaScript in a logged-in page, query browser sessions, use CDP fallback, recover browser bridge state, or validate browser automation through the local HTTP RPC API.
---

# TMWD CDP Bridge

Use this skill for real local Chromium-family browser work through
`tmwd-cdp-bridge`. It is generic Agent guidance and assumes only shell access,
file reads, and `curl` or equivalent HTTP.

## Non-Negotiables

- Use only `POST /v1/rpc` for RPC. Never call `/link`; it is intentionally unsupported.
- Do not use repository E2E scripts for real user tasks. Real tasks use the CLI server, `/health`, token file, and `/v1/rpc`.
- Keep all bridge traffic on `127.0.0.1`; do not expose bridge ports externally.
- Never print the full token, cookies, authorization headers, or unrelated page secrets in user-facing output.
- Use only extension ID `eghifjkffmcmffejmaaeicejpfopplem`. Old ID `aikfggdiblmijobpgdapacebmcjknbof` is incompatible and must stay disabled.
- Treat `execute_js` and `fallback:"cdp"` as code running in the user's browser page. Ask before destructive actions such as submitting forms, deleting data, changing account settings, purchases, or sending messages.

## Reference Files

- Read [references/install.md](references/install.md) when `tmwd-cdp-bridge` is missing, old, or not on `PATH`.
- Read [references/recovery.md](references/recovery.md) when `doctor --json` returns `degraded` or `fail`.
- Read [references/rpc.md](references/rpc.md) before making `/v1/rpc` calls or when choosing between normal JS, `fallback:"cdp"`, direct CDP, batch, or extension diagnostics.

## First Use Confirmation

Before the first browser action in a task, make sure the user intent covers
browser access. A concise confirmation is enough when intent is unclear:

`This can read the current page and execute JavaScript/CDP in your local browser. Should I continue?`

If the user explicitly asked to inspect, operate, or test a site, proceed and
avoid repeated confirmations for read-only actions.

## Start Or Reuse

Track whether you started the bridge in this task. Stop only a bridge you
started; reuse matching existing bridges.

1. If `tmwd-cdp-bridge` is missing or too old for `doctor`, read
   [references/install.md](references/install.md).
2. Run:

```sh
tmwd-cdp-bridge doctor --json
```

3. Branch on JSON fields, not process exit code:
   - `status:"ok"`: proceed to health/token/RPC.
   - `status:"degraded"`: follow only listed local `recovery` actions, then rerun `doctor --json`.
   - `status:"fail"`: do not proceed with page work; resolve install/version/port prerequisites first.
4. If `doctor` is unavailable because the installed binary is older, fall back to:

```sh
tmwd-cdp-bridge status --json
```

Use plain `tmwd-cdp-bridge doctor` only for human-readable troubleshooting.

## Ready Check

Proceed with browser page actions only when `/health` shows the expected bridge,
extension, and connection:

```sh
curl -s "http://127.0.0.1:${CDP_BRIDGE_HTTP_PORT:-18766}/health"
```

Required fields:

```json
{
  "server": "tmwd-cdp-bridge",
  "extension_connected": true,
  "extension_id": "eghifjkffmcmffejmaaeicejpfopplem"
}
```

If `extension_connected` is false, follow `doctor --json` recovery guidance.
If another service owns the port or the version mismatches, do not kill
unrelated processes automatically.

## Real Task Flow

1. Confirm user intent for browser access if needed.
2. Run `doctor --json`; recover using [references/recovery.md](references/recovery.md) until ready.
3. Confirm `/health` identity, fixed extension ID, and `extension_connected:true`.
4. Read token into a variable without printing it. Use [references/rpc.md](references/rpc.md) for platform helpers.
5. List or find sessions.
6. Execute read-only JavaScript first; use `fallback:"cdp"` only for CSP/page-world failures.
7. Summarize only task-relevant results.
8. Stop only the server you started for this task.

## Lifecycle Hooks

Some Agent hosts support activate/deactivate hooks. They are optional. Hook
behavior must match the shell flow:

- activate: run `doctor --json`; start only if no compatible bridge is running and recovery lists `START_BRIDGE`; never hide version/port conflicts.
- deactivate: stop only if this Agent started the bridge; otherwise leave it running.
