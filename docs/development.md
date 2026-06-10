# Development

This document is for maintainers and agents changing `tmwd-cdp-bridge`.

## Local Checks

Run these before handing off code changes:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
node --check extension/background.js
node --check scripts/e2e_chrome_smoke.mjs
node --check scripts/e2e_browser_inspect_url.mjs
node --check scripts/smoke_no_extension.mjs
node --check scripts/skill_minimal_flow.mjs
node --check scripts/soak_bridge.mjs
node --check scripts/smoke_real_browser.mjs
python3 scripts/validate_skill.py skills/tmwd-cdp-bridge
```

## CI Matrix

`.github/workflows/ci.yml` runs on macOS, Linux, and Windows:

- Rust formatting
- Clippy with warnings denied
- Unit and integration tests
- Node syntax checks for extension and harness scripts
- No-extension smoke covering start/status/stop, HTTP auth, and the `NO_EXTENSION` error path
- Repository-local Skill validation

Browser-backed smoke is intentionally not a required CI job because hosted CI browsers can be inconsistent with unpacked MV3 extension loading. Treat real browser smoke as a release gate.

## Harness Boundary

Development harnesses under `scripts/` are for local regression testing and release validation. They may start temporary browser profiles, temporary bridge processes, and local test pages.

Do not use these scripts for real user tasks or logged-in pages. Real tasks must use the installed or source-run CLI, the loaded extension, `/health`, token file, and authenticated `/v1/rpc`.

Useful harnesses:

- `scripts/smoke_real_browser.mjs`: starts a temporary browser and bridge, loads the real extension, and verifies normal JS, direct CDP, explicit CDP fallback, tab close, and extension reload/reconnect.
- `scripts/smoke_no_extension.mjs`: starts the debug binary without a browser extension and verifies cross-platform lifecycle and auth behavior.
- `scripts/skill_minimal_flow.mjs`: validates the generic Agent minimum flow using only CLI, token file, health, and RPC.
- `scripts/soak_bridge.mjs`: probes an already running bridge for a configurable duration and interval.
- `scripts/e2e_chrome_smoke.mjs`: protocol-level browser smoke using temporary resources.
- `scripts/e2e_browser_inspect_url.mjs`: development-only inspection harness for a target URL.

## Release Build

`.github/workflows/release.yml` builds release archives for:

- macOS arm64
- macOS x64
- Linux x64
- Windows x64

Each archive includes the binary, README, LICENSE, docs, and Skill directory. Each archive gets a `.sha256` checksum.

Tag releases with `vX.Y.Z` after updating `Cargo.toml`, README/API examples if needed, and the extension version when extension files change.

## Release Gate

Before publishing a production release:

1. Run the local checks above.
2. Run real browser smoke on macOS Edge:

```sh
node scripts/smoke_real_browser.mjs
```

3. Run Chrome as well when installed:

```sh
BROWSER_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" node scripts/smoke_real_browser.mjs
```

If Chrome does not connect from the temporary `--load-extension` profile, also
verify a manually installed Chrome profile:

```sh
cargo run -- install chrome
cargo run -- start
cargo run -- status --json
```

Then open `chrome://extensions`, load the repository `extension/` directory as
an unpacked extension, and confirm `status --json` reports
`extension_connected: true` for `eghifjkffmcmffejmaaeicejpfopplem`. Use the
normal token file plus `/v1/rpc` flow to verify at least one page read, one
direct CDP command, and one explicit `fallback:"cdp"` command.

4. Run the long soak against a real running bridge:

```sh
node scripts/soak_bridge.mjs --duration 30m --interval 60s
```

5. Confirm `/link` still returns `404`.
6. Confirm no logs, docs, or release notes contain a full bearer token, cookies, or authorization headers with secrets.

Record the release gate result in the release notes or the iteration plan with:

- commit/tag
- OS and browser used for real smoke
- soak duration and pass/fail counts
- CI run link
- known deferred items

## Regression Checklist

Use this checklist when changing protocol, extension, CLI lifecycle, or Skill behavior.

Protocol or server changes:

- `cargo test`
- `/health` identity and heartbeat fields
- unauthorized `/v1/rpc`
- malformed JSON `BAD_REQUEST`
- `/link` remains `404`
- timeout cleanup and extension disconnect behavior

Extension changes:

- `node --check extension/background.js`
- `node scripts/smoke_real_browser.mjs`
- direct CDP mode
- explicit `fallback:"cdp"`
- debugger detach after success and failure
- extension reload/reconnect

CLI lifecycle changes:

- `node scripts/smoke_no_extension.mjs`
- start with stale pid file
- stop refuses pid mismatch
- non-bridge port conflict is not killed
- version mismatch gives extension install guidance

Skill changes:

- `python3 scripts/validate_skill.py skills/tmwd-cdp-bridge`
- `node scripts/skill_minimal_flow.mjs`
- examples use `/v1/rpc`, never legacy `/link`
- examples do not print the full token

## Known Limits

- One active extension WebSocket is tracked at a time.
- Ports bind to `127.0.0.1` only.
- The extension is loaded unpacked; Chrome Web Store packaging is not part of this project.
- High-level Playwright-style APIs are intentionally out of scope.
- The CLI does not automatically restart browsers or kill non-owned processes.
- CI does not require real browser smoke; real Edge/Chrome extension smoke is a release gate.
