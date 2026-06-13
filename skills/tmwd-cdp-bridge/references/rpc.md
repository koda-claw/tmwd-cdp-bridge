# RPC Usage

Use this reference after `doctor --json` and `/health` confirm the bridge is
ready. Never call `/link`; use only authenticated `POST /v1/rpc`.

## Token Helpers

macOS/Linux:

```sh
APP_DIR="${CDP_BRIDGE_APP_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge}"
case "$(uname -s)" in
  Darwin) APP_DIR="${CDP_BRIDGE_APP_DIR:-$HOME/Library/Application Support/tmwd-cdp-bridge}" ;;
esac
TOKEN="$(cat "$APP_DIR/token")"
HTTP_PORT="${CDP_BRIDGE_HTTP_PORT:-18766}"
```

Windows PowerShell:

```powershell
$AppDir = if ($env:CDP_BRIDGE_APP_DIR) { $env:CDP_BRIDGE_APP_DIR } else { Join-Path $env:LOCALAPPDATA "tmwd-cdp-bridge" }
$Token = (Get-Content -Raw (Join-Path $AppDir "token")).Trim()
$HttpPort = if ($env:CDP_BRIDGE_HTTP_PORT) { $env:CDP_BRIDGE_HTTP_PORT } else { "18766" }
```

Never print `$TOKEN` or `$Token`.

## List Sessions

```sh
curl -s "http://127.0.0.1:${HTTP_PORT:-18766}/v1/rpc" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"cmd":"get_all_sessions","request_id":"sessions-1"}'
```

## Find A Tab

```json
{
  "cmd": "find_session",
  "request_id": "find-1",
  "url_contains": "example.com",
  "title_contains": "Dashboard"
}
```

## Read Page Data

Start with read-only JavaScript:

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

## CSP Or Page-World Failures

Retry with explicit CDP fallback only after normal page JS fails due to CSP or
isolated-world behavior:

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

## Batch Reads

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

Only after the new bridge is connected, list extensions through the new bridge:

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

If `aikfggdiblmijobpgdapacebmcjknbof` is enabled, tell the user to disable it
in the browser extension page. Do not use the old protocol.

## Common Errors

- `UNAUTHORIZED`: re-read the token file.
- `NO_EXTENSION`: server is up but extension is not connected; reload the extension and re-check health.
- `NO_SESSION`: list sessions and choose a current `sessionId`.
- `EXEC_TIMEOUT`: confirm the page is responsive before increasing timeout.
- `BAD_REQUEST`: check JSON shape, command spelling, and `fallback` value.
