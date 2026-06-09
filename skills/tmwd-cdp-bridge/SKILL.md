---
name: tmwd-cdp-bridge
description: Use tmwd-cdp-bridge to control a local Chrome or Edge browser through the bundled MV3 extension. Use when an agent needs to inspect real browser tabs, execute JavaScript in a logged-in page, query browser sessions, use CDP fallback, recover browser bridge state, or validate browser automation through the local HTTP RPC API.
---

# TMWD CDP Bridge

Use this skill for real local Chromium-family browser work through `tmwd-cdp-bridge`. It is generic Agent guidance: it assumes only shell access, file reads, and `curl` or equivalent HTTP.

## Non-Negotiables

- Use only `POST /v1/rpc` for RPC. Never call `/link`; it is intentionally unsupported.
- Do not use repository E2E scripts for real user tasks. Real tasks use the CLI server, `/health`, token file, and `/v1/rpc`.
- Keep all bridge traffic on `127.0.0.1`; do not expose bridge ports externally.
- Never print the full token, cookies, authorization headers, or unrelated page secrets in user-facing output.
- Use only extension ID `eghifjkffmcmffejmaaeicejpfopplem`. Old ID `aikfggdiblmijobpgdapacebmcjknbof` is incompatible and must stay disabled.
- Treat `execute_js` and `fallback:"cdp"` as code running in the user's browser page. Ask/confirm before destructive actions such as submitting forms, deleting data, changing account settings, purchases, or sending messages.

## First Use Confirmation

Before the first browser action in a task, make sure the user intent covers browser access. A concise confirmation is enough when intent is unclear:

`This can read the current page and execute JavaScript/CDP in your local browser. Should I continue?`

If the user explicitly asked to inspect, operate, or test a site, proceed and avoid repeated confirmations for read-only actions.

## Start Or Reuse

If `tmwd-cdp-bridge` is not on `PATH` and you are inside this source checkout, replace it with `cargo run --`.

Track whether you started the bridge in this task. Stop only a bridge you started; reuse matching existing bridges.

```sh
tmwd-cdp-bridge status
```

If no compatible server is running:

```sh
tmwd-cdp-bridge start
```

If install or version errors appear:

```sh
tmwd-cdp-bridge install edge
# or
tmwd-cdp-bridge install chrome
```

Load the printed `extension/` directory in `edge://extensions` or `chrome://extensions` with Developer mode enabled. Use `tmwd-cdp-bridge repair edge` or `repair chrome` to reprint recovery instructions.

## Token And Health

Default token files:

- macOS: `~/Library/Application Support/tmwd-cdp-bridge/token`
- Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge/token`
- Windows: `%LOCALAPPDATA%\tmwd-cdp-bridge\token`

Portable shell helper for macOS/Linux:

```sh
APP_DIR="${CDP_BRIDGE_APP_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge}"
case "$(uname -s)" in
  Darwin) APP_DIR="${CDP_BRIDGE_APP_DIR:-$HOME/Library/Application Support/tmwd-cdp-bridge}" ;;
esac
TOKEN="$(cat "$APP_DIR/token")"
HTTP_PORT="${CDP_BRIDGE_HTTP_PORT:-18766}"
```

Health:

```sh
curl -s "http://127.0.0.1:${HTTP_PORT:-18766}/health"
```

Proceed with page actions only when health shows:

```json
{
  "server": "tmwd-cdp-bridge",
  "extension_connected": true,
  "extension_id": "eghifjkffmcmffejmaaeicejpfopplem"
}
```

If `extension_connected` is false, open/reload the extension and re-check health. If another server is on the port or the version mismatches, follow the CLI error; do not kill unrelated processes.

## Real Task Flow

1. Confirm user intent for browser access if needed.
2. `status`; reuse a matching server or start one and remember you started it.
3. Confirm `/health` identity, fixed extension ID, and `extension_connected:true`.
4. Read token into a variable without printing it.
5. List or find sessions.
6. Execute read-only JavaScript first; use `fallback:"cdp"` only for CSP/page-world failures.
7. Summarize only task-relevant results.
8. Stop only the server you started for this task.

## RPC Examples

All RPC calls:

```sh
curl -s "http://127.0.0.1:${HTTP_PORT:-18766}/v1/rpc" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"cmd":"get_all_sessions","request_id":"sessions-1"}'
```

Find a tab:

```json
{
  "cmd": "find_session",
  "request_id": "find-1",
  "url_contains": "example.com",
  "title_contains": "Dashboard"
}
```

Read page data:

```json
{
  "cmd": "execute_js",
  "request_id": "read-1",
  "sessionId": "123",
  "code": "({title: document.title, url: location.href, text: document.body.innerText.slice(0, 4000)})",
  "timeout": 15
}
```

Useful snippets:

```js
document.title
location.href
document.body.innerText.slice(0, 4000)
Array.from(document.querySelectorAll('a,button,[role=button]')).map((e,i)=>({i,text:e.innerText?.trim(),href:e.href||null})).filter(x=>x.text||x.href).slice(0,100)
```

CDP fallback:

```json
{
  "cmd": "execute_js",
  "request_id": "fallback-1",
  "sessionId": "123",
  "fallback": "cdp",
  "code": "document.title",
  "timeout": 15
}
```

Direct CDP:

```json
{
  "cmd": "execute_js",
  "request_id": "cdp-1",
  "sessionId": "123",
  "mode": "cdp",
  "code": {
    "method": "Runtime.evaluate",
    "params": {
      "expression": "document.title",
      "awaitPromise": true,
      "returnByValue": true
    }
  }
}
```

Batch read:

```json
{
  "cmd": "batch",
  "request_id": "batch-1",
  "items": [
    {"cmd":"execute_js","request_id":"title","sessionId":"123","code":"document.title"},
    {"cmd":"execute_js","request_id":"url","sessionId":"123","code":"location.href"}
  ]
}
```

Check each batch item for `error`.

## Old Extension Diagnostic

If the browser may have both old and new extensions, list extensions through the new bridge after it is connected:

```json
{
  "cmd": "execute_js",
  "request_id": "extensions-1",
  "code": {
    "cmd": "management",
    "method": "list"
  }
}
```

If `aikfggdiblmijobpgdapacebmcjknbof` is enabled, tell the user to disable it in the browser extension page. Do not use the old protocol.

## Optional Lifecycle Hooks

Some Agent hosts support activate/deactivate hooks. They are optional. Hook behavior must be equivalent to the shell flow:

- activate: run `status`; start only if no compatible bridge is running; never hide version/port conflicts.
- deactivate: stop only if this Agent started the bridge; otherwise leave it running.

Agents without hooks should use the shell commands above.

## Troubleshooting

- `UNAUTHORIZED`: re-read the token file.
- `NO_EXTENSION`: server is up but extension is not connected; reload the extension and re-check health.
- `NO_SESSION`: list sessions and choose a current `sessionId`.
- `EXEC_TIMEOUT`: confirm the page is responsive before increasing timeout.
- `BAD_REQUEST`: check JSON shape, command spelling, and `fallback` value.
- Port conflict: set both `CDP_BRIDGE_WS_PORT` and `CDP_BRIDGE_HTTP_PORT`; do not silently switch only one side.
