# Troubleshooting

## Check Status First

```sh
tmwd-cdp-bridge status
tmwd-cdp-bridge status --json
curl -s http://127.0.0.1:18766/health
```

If working from source, replace `tmwd-cdp-bridge` with `cargo run --`. Use
`status` for a readable summary and `status --json` for scripts or agents.

## Port Is In Use

`start` refuses to kill unknown processes. If the default ports are busy, either stop the conflicting process yourself or choose explicit ports:

```sh
CDP_BRIDGE_WS_PORT=28765 CDP_BRIDGE_HTTP_PORT=28766 tmwd-cdp-bridge start
```

If custom ports are used, the extension must use the same WebSocket and health URLs. For development harnesses this is patched automatically. For normal use, prefer the default ports unless there is a conflict.

## Extension Is Not Connected

Symptoms:

- `/health` has `"extension_connected": false`
- RPC `execute_js` returns `NO_EXTENSION`

Fix:

1. Run `tmwd-cdp-bridge install edge` or `tmwd-cdp-bridge install chrome`.
2. Open `edge://extensions` or `chrome://extensions`.
3. Enable Developer mode.
4. Load the printed `extension/` directory.
5. Reload the extension.
6. Re-check `/health`.

The expected extension ID is `eghifjkffmcmffejmaaeicejpfopplem`.

The old extension ID `aikfggdiblmijobpgdapacebmcjknbof` is incompatible. Keep it disabled.

## Version Mismatch

`start` validates the installed extension version file before listening. If it reports a mismatch:

```sh
tmwd-cdp-bridge upgrade
```

Then reload the unpacked extension in the browser.

## Token Missing Or Unauthorized

The bearer token lives in the platform app data directory:

- macOS: `~/Library/Application Support/tmwd-cdp-bridge/token`
- Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge/token`
- Windows: `%LOCALAPPDATA%\tmwd-cdp-bridge\token`

Read it into a shell variable instead of printing it:

```sh
TOKEN="$(cat "$HOME/Library/Application Support/tmwd-cdp-bridge/token")"
```

If the token file is missing, start the bridge once so it can create the token.

## CDP Or Page Execution Fails

For normal page inspection, start with plain `execute_js`.

If the page blocks page-world evaluation or returns CSP-related errors, retry with explicit CDP fallback:

```json
{
  "cmd": "execute_js",
  "sessionId": "123",
  "fallback": "cdp",
  "code": "document.title"
}
```

For direct Chrome DevTools Protocol commands, use `mode: "cdp"`:

```json
{
  "cmd": "execute_js",
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

## Stop Refuses To Run

`stop` only shuts down a verified `tmwd-cdp-bridge` process whose `/health` pid matches the pid file. This prevents killing unrelated processes.

If a pid file is stale, `start` removes it before writing the new pid. If a pid mismatch persists, inspect the app dir and running process manually before deleting files.

## Legacy `/link`

`/link` is intentionally unsupported and should return `404`. Use `POST /v1/rpc`.
