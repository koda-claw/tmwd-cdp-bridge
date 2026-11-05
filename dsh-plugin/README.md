# tmwd-cdp-bridge DSH plugin

Optional [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)
plugin that exposes the local `tmwd-cdp-bridge` browser bridge as native model
tools. The plugin is a thin authenticated HTTP client: the bridge, the
Chrome/Edge extension, and the browser still perform all browser work.

## Tools

| Tool | Arguments | Purpose |
|---|---|---|
| `tmwd_browser_status` | none | Check `/health`, extension connectivity, and token presence. Safe and read-only. |
| `tmwd_list_tabs` | none | List known tabs with `id`, URL, title, and active state. |
| `tmwd_find_tab` | `url_contains?`, `title_contains?` | Find the first tab matching at least one filter. |
| `tmwd_execute_js` | `session_id?`, `code`, `timeout?`, `fallback?` | Evaluate JavaScript in a tab and return a JSON-serializable value. |
| `tmwd_cdp` | `session_id?`, `method`, `params?`, `timeout?` | Send a raw Chrome DevTools Protocol command for CDP-only work or CSP failures. |

All tools read the bridge token file on each RPC call. The token value is never
returned by a tool or written to logs.

## Requirements

- DeepSeek Harness `0.2.2-rc.1` or newer with the `tools` service.
- `tmwd-cdp-bridge` installed and running on the same machine.
- The matching unpacked extension loaded in Chrome or Edge and connected.

Check the last two before installing the plugin:

```sh
tmwd-cdp-bridge status --json
tmwd-cdp-bridge start
curl -s http://127.0.0.1:18766/health
```

`/health` must report `"extension_connected": true` and extension id
`eghifjkffmcmffejmaaeicejpfopplem`.

## Install

From a source checkout:

```sh
dsh plugin --profile web add ./dsh-plugin
dsh web
```

The CLI records the dependency, links the bundle patch in
`dsh-plugin/cordis.patch.yml`, and enables the bundle in the profile. If the
profile is already running with HMR, the new tools appear without a restart;
otherwise restart the profile.

The plugin is not published to npm. Install it from this checkout or from a
release archive that includes `dsh-plugin/`. The package name is
`dsh-plugin-tmwd-cdp-bridge`; the `dsh-plugin-` prefix follows the naming
convention used by community DSH bundles.

## Configuration

The bundle inserts entry id `tmwd-cdp-bridge`. Override it in the profile's own
`cordis.patch.yml` (for example `~/.dsh/profiles/web/cordis.patch.yml`):

```yaml
- id: tmwd-cdp-bridge
  config:
    httpPort: 18766
    requestTimeoutMs: 120000
    # tokenFile: /absolute/path/to/token
    # appDir: /absolute/path/to/tmwd-cdp-bridge
```

| Field | Default | Meaning |
|---|---|---|
| `httpPort` | `18766` | Bridge HTTP RPC port. The host is always `127.0.0.1`. |
| `requestTimeoutMs` | `120000` | Upper bound for one HTTP request. `tmwd_execute_js` and `tmwd_cdp` extend it when `timeout` plus overhead is larger. |
| `tokenFile` | platform default | Explicit token path. |
| `appDir` | platform default | Bridge application directory; the plugin reads `<appDir>/token`. |

Token path defaults match the bridge:

- macOS: `~/Library/Application Support/tmwd-cdp-bridge/token`
- Linux: `${XDG_DATA_HOME:-$HOME/.local/share}/tmwd-cdp-bridge/token`
- Windows: `%LOCALAPPDATA%\tmwd-cdp-bridge\token`

`CDP_BRIDGE_APP_DIR` is honored when `appDir` is not set.

## Typical flow

1. Call `tmwd_browser_status` and fix reachability or extension connectivity if
   needed.
2. Call `tmwd_list_tabs` or `tmwd_find_tab` to choose a tab.
3. Start with read-only `tmwd_execute_js`, for example `document.title` or
   `document.body.innerText.slice(0, 4000)`.
4. On CSP or isolated-world failures, retry `tmwd_execute_js` with
   `fallback: "cdp"`, or call `tmwd_cdp` with `Runtime.evaluate`.
5. Ask the user before destructive page actions.

## Security and privacy

- The plugin only talks to `http://127.0.0.1:<httpPort>`; arbitrary bridge URLs
  are not configurable.
- The token is read from disk per call and sent only to the loopback bridge.
  Errors never include the token.
- `tmwd_execute_js` and `tmwd_cdp` run in the user's logged-in browser context.
  They can read page data and change page state. Tool descriptions instruct the
  model to confirm destructive actions, but the plugin itself cannot enforce
  user intent.
- The plugin does not start, stop, install, or shut down the bridge, and does
  not expose the bridge ports.
- The bridge and extension hold powerful browser permissions (`cookies`,
  `debugger`, `<all_urls>` in the extension). Treat the token file as browser
  control and install the bridge only from a trusted source.

## Limitations

- The bridge must already be running with a connected extension; there is no
  auto-start.
- One bridge process supports one browser connection.
- `execute_js` targets the top-level frame; CDP mode covers CSP and
  isolated-world cases that normal evaluation cannot.
- Tab creation, tab closing, and the bridge `batch` and `shutdown` commands are
  intentionally not exposed in this version.
- Validated against DeepSeek Harness `0.2.2-rc.1`, bridge `0.1.3`, and extension
  `2.3` on macOS. The Windows and Linux paths use the bridge's documented token
  locations; they are covered by unit tests but not by a real-browser release
  matrix yet.

## Why a native plugin instead of an MCP server

A native Cordis bundle registers tools directly and follows the profile's
configuration and token handling without another long-running process. An MCP
wrapper would be useful for agents outside DSH, but this repository already
ships the generic `skills/tmwd-cdp-bridge` flow and the RPC contract for those
hosts.

## Development

```sh
cd dsh-plugin
node --test
```

The unit tests cover token and path resolution, HTTP error mapping, timeouts,
and every tool's payload. The plugin was also smoke-tested against a real
DeepSeek Harness `0.2.2-rc.1` runtime with the five tools registered through
`defineTool`, and against a real bridge `0.1.3` with a connected extension
(status, tab listing, tab search, and a read-only `document.title` evaluation).
