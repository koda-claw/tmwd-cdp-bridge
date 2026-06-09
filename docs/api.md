# tmwd-cdp-bridge API Contract

This document is the stable contract for agents using `tmwd-cdp-bridge`.

## Transport

- Health: `GET http://127.0.0.1:18766/health`
- RPC: `POST http://127.0.0.1:18766/v1/rpc`
- RPC auth: `Authorization: Bearer <token>`
- Token file:
  - macOS: `~/Library/Application Support/tmwd-cdp-bridge/token`
  - Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge/token`
  - Windows: `%LOCALAPPDATA%\tmwd-cdp-bridge\token`
- Legacy `/link` is not supported and should return `404`.

All network access is local-only. Agents must not expose these ports externally.

## Health

`GET /health` is unauthenticated and only reports bridge identity and extension connection state.

Example:

```json
{
  "server": "tmwd-cdp-bridge",
  "version": "0.1.0",
  "pid": 12345,
  "extension_id": "eghifjkffmcmffejmaaeicejpfopplem",
  "allowed_extension_origin": "chrome-extension://eghifjkffmcmffejmaaeicejpfopplem",
  "extension_connected": true,
  "extension_connected_at_unix_ms": 1780980000000,
  "extension_last_seen_at_unix_ms": 1780980000123,
  "extension_last_seen_age_ms": 42
}
```

`extension_connected: true` means an authenticated extension WebSocket is currently attached. It does not guarantee a target page exists.

`extension_connected_at_unix_ms` is set while the extension WebSocket is attached. `extension_last_seen_at_unix_ms` and `extension_last_seen_age_ms` update when the bridge receives extension messages such as `ext_ready`, `tabs_update`, `ping`, `ack`, `result`, or `error`. When disconnected, `extension_connected_at_unix_ms` is `null`; `extension_last_seen_at_unix_ms` may retain the last observed timestamp for diagnostics.

## RPC Envelope

All successful HTTP-level RPC responses are wrapped in `r`.

Success:

```json
{
  "r": {
    "request_id": "req-1",
    "data": {},
    "newTabs": []
  }
}
```

Error:

```json
{
  "r": {
    "request_id": "req-1",
    "error": {
      "code": "NO_SESSION",
      "message": "no matching session"
    }
  }
}
```

`request_id` is echoed when provided. If omitted or impossible to parse, the server generates one.

## Commands

### `get_all_sessions`

Returns all tabs known from the connected extension.

Request:

```json
{"cmd":"get_all_sessions","request_id":"sessions-1"}
```

Response `data`:

```json
[
  {
    "id": 123,
    "url": "https://example.com/",
    "title": "Example",
    "active": true,
    "window_id": 1
  }
]
```

### `find_session`

Returns the first tab matching all provided filters.

Request:

```json
{
  "cmd": "find_session",
  "request_id": "find-1",
  "url_contains": "example.com",
  "title_contains": "Dashboard"
}
```

`browser` is accepted for forward compatibility but currently does not filter.

### `execute_js`

Runs JavaScript or a CDP command against a selected tab.

Request:

```json
{
  "cmd": "execute_js",
  "request_id": "exec-1",
  "sessionId": "123",
  "code": "document.title",
  "timeout": 15
}
```

Selection:

- `sessionId` or `tabId` selects a specific tab.
- Without a tab id, the server picks the active scriptable tab, then the last successful tab, then the first scriptable tab.
- Scriptable tabs are `http://` or `https://`.

`timeout` is per `execute_js` request in seconds. Default is `15`.

### CDP fallback

`fallback: "cdp"` keeps normal JavaScript execution as the first path. The extension may use CDP only when normal execution fails due to page-world or CSP limitations.

```json
{
  "cmd": "execute_js",
  "request_id": "exec-cdp-fallback-1",
  "sessionId": "123",
  "fallback": "cdp",
  "code": "document.title"
}
```

### Direct CDP mode

`mode: "cdp"` sends a Chrome DevTools Protocol command object to the extension.

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

The bridge forwards this to the extension as:

```json
{
  "cmd": "cdp",
  "method": "Runtime.evaluate",
  "params": {}
}
```

### `batch`

Executes `items` in order and returns per-item results. Items continue after a prior item fails.

Request:

```json
{
  "cmd": "batch",
  "request_id": "batch-1",
  "items": [
    {"cmd":"execute_js","request_id":"title","code":"document.title"},
    {"cmd":"execute_js","request_id":"url","code":"location.href"}
  ]
}
```

Response:

```json
{
  "r": {
    "request_id": "batch-1",
    "items": [
      {"request_id":"title","data":"Example"},
      {"request_id":"url","data":"https://example.com/"}
    ]
  }
}
```

Timeout semantics are per item. There is currently no total batch timeout.

### `shutdown`

Requests the running bridge process to exit.

Use this only for a bridge process started by the current Agent/session.

```json
{"cmd":"shutdown","request_id":"shutdown-1"}
```

## Error Codes

Public, stable error codes:

| Code | HTTP status | Meaning |
|------|-------------|---------|
| `UNAUTHORIZED` | 401 | Missing or invalid bearer token |
| `BAD_REQUEST` | 400 or 200 | Malformed JSON, unknown command, unsupported fallback, missing/invalid `code` |
| `NO_EXTENSION` | 200 | No authenticated extension WebSocket is connected |
| `NO_SESSION` | 200 | No usable tab, no matching tab, or invalid/nonexistent `sessionId` |
| `EXEC_TIMEOUT` | 200 | Extension did not respond before the per-item timeout |
| `EXEC_ERROR` | 200 | Extension returned an execution error |
| `INTERNAL` | 200 | Unexpected bridge-side execution failure |

Reserved error codes:

| Code | Status |
|------|--------|
| `TAB_CLOSED` | Reserved for a future explicit tab-closed path |
| `CDP_UNAVAILABLE` | Reserved for a future explicit CDP attach/permission path |
| `PORT_IN_USE` | Reserved for a future HTTP RPC representation; start currently reports this as a CLI error |

Agents should handle unknown future error codes by surfacing `message` and avoiding retries unless the action is safe.

## Compatibility Notes

- Fixed extension ID: `eghifjkffmcmffejmaaeicejpfopplem`.
- Legacy extension ID `aikfggdiblmijobpgdapacebmcjknbof` is incompatible.
- `mode=cdp`, `fallback=cdp`, and `batch` are part of the current contract.
- Browser control is intentionally low-level; this is not a Playwright-compatible API.
