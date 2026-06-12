# Doctor And Recovery Iteration Plan

## Goal

Add a first-class diagnostic and recovery workflow so any Agent or human can run
one command, understand the local bridge state, and know the next safe action.

The target command is:

```sh
tmwd-cdp-bridge doctor
tmwd-cdp-bridge doctor --json
```

`doctor` is read-only. It must never kill processes, rewrite tokens, reinstall
extensions, open browsers, or mutate browser state. It diagnoses and recommends.

## Scope

- Add a new CLI subcommand `doctor`.
- Support human-readable output and stable `--json` output.
- Check local runtime state across macOS, Linux, and Windows.
- Detect common first-use and recovery failures.
- Emit explicit recovery actions that generic Agents can follow.
- Update the bundled Skill to prefer `doctor --json` before browser work.
- Add tests for the JSON contract and major failure states.
- Keep `status` behavior stable; `doctor` complements it rather than replacing
  it.
- Keep diagnostics local-only and bounded by short timeouts.

Out of scope:

- Automatic browser extension installation into Chrome/Edge.
- Killing unknown processes or closing browser instances.
- Any internet access during diagnostics.
- Playwright-style high-level browser automation APIs.
- Web Store packaging.
- Removing legacy `/link` or legacy request-id compatibility beyond current
  behavior.

## Proposed User Experience

Human output should be compact, grouped, and action-oriented:

```text
tmwd-cdp-bridge doctor

Runtime
  Binary version: 0.1.3
  App dir: /Users/me/Library/Application Support/tmwd-cdp-bridge
  Token: present
  Installed extension copy: present, version 2.3

Bridge
  HTTP: running on 127.0.0.1:18766, version 0.1.3
  Extension: not connected

Next actions
  1. Open edge://extensions or chrome://extensions.
  2. Reload extension id eghifjkffmcmffejmaaeicejpfopplem.
  3. Re-run: tmwd-cdp-bridge doctor
```

JSON output should be stable enough for Agents:

```json
{
  "schema_version": 1,
  "status": "degraded",
  "summary": "Bridge is running but the extension is not connected.",
  "checks": [
    {
      "id": "extension_connection",
      "status": "fail",
      "message": "No extension WebSocket is connected.",
      "required": false,
      "kind": "readiness",
      "recovery": ["RELOAD_EXTENSION", "LOAD_UNPACKED_EXTENSION"]
    }
  ],
  "recovery": ["RELOAD_EXTENSION", "LOAD_UNPACKED_EXTENSION"]
}
```

Top-level `status` values:

- `ok`: all prerequisite and readiness checks pass.
- `degraded`: install state is coherent but the bridge is not ready yet, such as
  not running or extension disconnected.
- `fail`: core prerequisites are missing, mismatched, unreadable, or conflicting.

Aggregation rules:

- Any `prerequisite` check with `fail` makes the top-level status `fail`.
- Otherwise, any `prerequisite` check with `warn`, any `readiness` check with
  `fail` or `warn`, or a stopped bridge state makes the top-level status
  `degraded`.
- Otherwise, top-level status is `ok`.
- `unknown` should not hide a required failure. Use it only when a check is
  advisory or cannot be determined without mutating local/browser state.
- A `prerequisite` check may be `unknown` only when its prerequisite cannot be
  evaluated because an earlier state is absent, such as no reachable health
  endpoint. That does not make the top-level status `fail` by itself.

Check-level `status` values:

- `ok`
- `warn`
- `fail`
- `unknown`

Required top-level JSON fields:

- `schema_version`: numeric contract version, starting at `1`.
- `status`: one of `ok`, `degraded`, or `fail`.
- `summary`: short human-readable summary.
- `checks`: ordered array of check objects.
- `recovery`: ordered, deduplicated array of recovery action ids.

Required check fields:

- `id`: stable lowercase check id.
- `status`: one of `ok`, `warn`, `fail`, or `unknown`.
- `message`: short human-readable result.
- `required`: boolean. This mirrors `kind:"prerequisite"` in schema v1.
- `kind`: one of `prerequisite`, `readiness`, or `advisory`.
- `recovery`: ordered array of recovery action ids for that check.

Schema v1 readiness check ids:

- `token_file`
- `health_endpoint`
- `extension_connection`

A readiness check may be `required:false` and `status:"fail"`. That produces a
top-level `degraded` report when all required checks pass, because the bridge is
installed coherently but not ready for page work yet.

Optional check fields:

- `details`: small JSON object with non-secret diagnostic fields, such as
  expected version, installed version, port, pid, app dir, or extension id.

Do not include nondeterministic timestamps in the initial JSON contract unless a
later schema version needs them.

CLI exit code policy:

- `doctor` should exit `0` when diagnostics completed, even if the top-level
  report status is `degraded` or `fail`.
- It should exit non-zero only when the diagnostic command itself cannot run,
  for example invalid CLI arguments or an unexpected internal error.
- Agents must branch on the JSON `status` and `recovery` fields, not on the
  process exit code.

Runtime behavior constraints:

- `doctor` may read local files, inspect local paths, probe configured
  `127.0.0.1` ports, and call local `/health`.
- `doctor` must not make internet requests, contact GitHub Releases, or call
  external browser/profile services.
- Network probes should use short timeouts. The full command should normally
  complete within two seconds when local ports are slow or unreachable.

## Recovery Actions

Use stable uppercase action ids so Skills and Agents can branch on them.

Initial set:

- `START_BRIDGE`: run `tmwd-cdp-bridge start`.
- `RUN_INSTALL_EDGE`: run `tmwd-cdp-bridge install edge`.
- `RUN_INSTALL_CHROME`: run `tmwd-cdp-bridge install chrome`.
- `RUN_INSTALL_BROWSER`: run either `install edge` or `install chrome`, choosing
  the browser the user intends to use.
- `LOAD_UNPACKED_EXTENSION`: load the printed extension directory in
  `edge://extensions` or `chrome://extensions`.
- `RELOAD_EXTENSION`: reload extension id
  `eghifjkffmcmffejmaaeicejpfopplem`.
- `DISABLE_LEGACY_EXTENSION`: disable old extension id
  `aikfggdiblmijobpgdapacebmcjknbof`.
- `STOP_CONFLICTING_PROCESS`: stop a non-bridge process that owns the configured
  HTTP or WebSocket port.
- `USE_DIFFERENT_PORT`: set `CDP_BRIDGE_HTTP_PORT` or `CDP_BRIDGE_WS_PORT`.
- `UPGRADE_BINARY`: run `tmwd-cdp-bridge upgrade`.
- `FIX_TOKEN_FILE`: restore a missing/unreadable token file by starting the
  bridge once or fixing file ownership/permissions. Do not print the token.
- `REPAIR_INSTALL`: run `tmwd-cdp-bridge repair edge` or
  `tmwd-cdp-bridge repair chrome` to reprint recovery guidance.

Recovery arrays should be deduplicated and ordered by the most likely next
step. Human output may translate these action ids into prose.

When `doctor` cannot know the user's intended browser, it should prefer
`RUN_INSTALL_BROWSER` in JSON and print both concrete Edge and Chrome command
examples in human output. Use browser-specific actions only when the user
selected a browser or a browser-specific failure is known.

## Checks

Initial stable check ids:

- `binary_version`
- `app_dir`
- `app_dir_permissions`
- `token_file`
- `token_permissions`
- `extension_copy`
- `extension_version`
- `pid_file`
- `http_port`
- `health_endpoint`
- `server_identity`
- `server_version`
- `pid_match`
- `ws_port`
- `extension_id`
- `extension_origin`
- `extension_connection`
- `browser_detection`

Schema v1 check classification:

| Check id | Required | Kind |
| --- | --- | --- |
| `binary_version` | true | prerequisite |
| `app_dir` | true | prerequisite |
| `app_dir_permissions` | false | advisory |
| `token_file` | false | readiness |
| `token_permissions` | false | advisory |
| `extension_copy` | true | prerequisite |
| `extension_version` | true | prerequisite |
| `pid_file` | false | advisory |
| `http_port` | true | prerequisite |
| `health_endpoint` | false | readiness |
| `server_identity` | true | prerequisite |
| `server_version` | true | prerequisite |
| `pid_match` | false | advisory |
| `ws_port` | false | advisory |
| `extension_id` | true | prerequisite |
| `extension_origin` | true | prerequisite |
| `extension_connection` | false | readiness |
| `browser_detection` | false | advisory |

`Kind` controls aggregation and messaging:

- `prerequisite`: required for a ready local bridge.
- `readiness`: required for browser page work but recoverable without changing
  the installed CLI or app-dir prerequisites.
- `advisory`: useful diagnostics that should not make the top-level status
  `fail` by themselves.

### Runtime Checks

- Binary version from `env!("CARGO_PKG_VERSION")`.
- Platform app dir resolution.
- App dir exists and is a directory.
- Token file exists.
- Token file is readable.
- On Unix, token/app dir permissions are private enough.
- Extension copy exists under the app dir.
- Installed extension version file exists.
- Installed extension version equals `EXTENSION_VERSION`.
- Pid file exists or is absent.
- Pid file contains a parseable pid when present.

The token file is created by `start` when missing. If the bridge is stopped and
the token file is missing, report `token_file` as a readiness failure with
`START_BRIDGE` and `FIX_TOKEN_FILE`, not as a prerequisite failure. If the bridge
is running but the token file is missing or unreadable, keep the top-level status
`degraded` and recommend `FIX_TOKEN_FILE`; do not print or regenerate the token
from `doctor`.

### Port And Server Checks

- Configured HTTP port is reachable or free.
- Reachable HTTP server exposes `/health`.
- `/health.server` is `tmwd-cdp-bridge`.
- Running server version matches the current binary.
- Running server pid matches the pid file when both are present.
- Configured WebSocket port is not clearly occupied by a non-bridge process
  when the HTTP side is not a bridge.

Port availability checks are advisory and race-prone by nature. They should be
used to produce recovery guidance, not as proof that a later `start` call cannot
fail.

When no HTTP server is reachable, `server_identity` and `server_version` may be
`unknown` and should not make the report fail. When an HTTP server is reachable
but is not `tmwd-cdp-bridge`, or is `tmwd-cdp-bridge` with an incompatible
version, those checks should be `fail` and the top-level status should be
`fail`.

### Extension Checks

- `/health.extension_id` equals `eghifjkffmcmffejmaaeicejpfopplem`.
- `/health.allowed_extension_origin` equals the expected origin unless an
  explicit test override is in use.
- `/health.extension_connected` is true for ready state.
- If extension is disconnected, recommend reload/load actions.
- If the installed version file is missing or mismatched, recommend install and
  extension reload actions before page work.
- `doctor` cannot reliably inspect all installed browser extensions without
  browser automation. It may include `DISABLE_LEGACY_EXTENSION` as advisory
  guidance when the extension is disconnected or the user is in manual browser
  recovery, but it must not claim the legacy extension is installed unless a
  future browser-backed check proves it.

When no compatible bridge health response is available, `extension_id` and
`extension_origin` may be `unknown`. When a compatible bridge health response is
available but reports an unexpected extension id or origin, treat that as a
prerequisite failure because the CLI should not guide Agents to use a bridge with
an unexpected extension trust boundary.

### Browser Guidance Checks

`doctor` should not require Chrome or Edge to be installed to pass core bridge
diagnostics. Browser detection is advisory:

- On macOS, detect common Chrome and Edge app paths when available.
- On Linux, detect common `google-chrome`, `chromium`, `microsoft-edge`
  executables when available.
- On Windows, detect common install paths or report `unknown` if not cheaply
  discoverable.
- If neither browser is detected, suggest install docs but do not fail the
  bridge checks solely for that reason.

## Implementation Steps

1. Define the diagnostic model
   - Files: `src/doctor.rs`, `src/lib.rs`
   - Add serializable structs for top-level report, checks, and recovery
     actions.
   - Keep action ids as explicit string constants or an enum with stable serde
     names.
   - Verify: unit tests for status aggregation and recovery deduplication.

2. Add CLI command wiring
   - Files: `src/cli.rs`
   - Add `doctor` and `doctor --json`.
   - Human output should not print the full token or authorization headers.
   - JSON output should be deterministic enough for snapshot-like assertions.
   - Verify: integration tests invoke the binary.

3. Implement read-only runtime checks
   - Files: `src/doctor.rs`, maybe reuse helpers from `src/config.rs`
   - Check app dir, token, extension copy, version file, pid file, and
     permissions.
   - Do not create app dir or token as a side effect.
   - Verify: temp-dir tests for empty, partial, and valid app dirs.

4. Implement port and health checks
   - Files: `src/doctor.rs`
   - Probe `/health` with a short timeout.
   - Distinguish unreachable, non-bridge response, bridge version mismatch,
     and compatible bridge states.
   - Keep WebSocket port conflict detection conservative. A TCP connection or
     bind probe may identify that something is listening, but `doctor` should
     label ownership as `unknown` unless HTTP `/health` proves a bridge.
   - Verify: integration tests using the existing tiny HTTP server helper and a
     real bridge process.

5. Implement extension readiness checks
   - Files: `src/doctor.rs`
   - Read `/health` fields and classify extension disconnected versus ready.
   - Map missing/mismatched installed extension version to install/reload
     recovery actions.
   - Verify: integration tests with a running bridge before and after mock
     extension WebSocket connection.

6. Update docs and Skill
   - Files: `README.md`, `docs/troubleshooting.md`,
     `skills/tmwd-cdp-bridge/SKILL.md`
   - Make `doctor --json` the first recovery step for Agents.
   - Keep `status --json` as a lightweight status command, but recommend
     `doctor --json` when anything is not ready.
   - Verify: `python3 scripts/validate_skill.py skills/tmwd-cdp-bridge`.

7. Add release note requirements
   - Files: next `docs/releases/vX.Y.Z.md`
   - Mention `doctor`, JSON contract, and recovery actions when the feature is
     released.

## Risks And Mitigations

| Risk | Mitigation |
| --- | --- |
| `doctor` accidentally mutates runtime state | Keep checks read-only; do not call `ensure_app_dir`, token creation, install, start, stop, or repair internally. |
| JSON contract churn breaks Agent integrations | Define stable action ids and check ids up front; add integration tests for representative JSON fields. |
| Human output leaks token or secrets | Report token presence, readability, and optional short prefix only if already accepted elsewhere; never print full token, cookies, or auth headers. |
| Port checks misidentify unrelated services | Treat unknown services as non-bridge conflicts and recommend manual stop/custom port; never kill processes. |
| Cross-platform browser detection is flaky | Keep browser detection advisory; only fail required bridge readiness checks. |
| `doctor` overlaps confusingly with `status` | Keep `status` as concise current state; make `doctor` a diagnostic report with recovery actions. |
| Running bridge has older version but still works | Report version mismatch clearly and recommend `UPGRADE_BINARY` or stopping the older bridge; do not auto-stop it. |
| Exit code is misused by Agents | Exit `0` for completed diagnostics and require Agents to inspect JSON `status`; reserve non-zero for command/internal failures. |
| Legacy extension conflict is overclaimed | Treat `DISABLE_LEGACY_EXTENSION` as advisory unless a future browser-backed check can prove the old extension is installed. |
| Diagnostics hang on slow local ports | Use short localhost probe timeouts and test an unreachable-port path. |
| Diagnostics reach external networks | Keep `doctor` local-only; do not check GitHub, browser stores, or remote update metadata. |
| Tests become slow or require real browsers | Unit/integration tests should not require real Chrome/Edge; real browser smoke remains release gate. |

## Acceptance Criteria

- `tmwd-cdp-bridge doctor` exists.
- `tmwd-cdp-bridge doctor --json` exists.
- `doctor` does not create, delete, or rewrite app-dir files.
- `doctor` does not make internet requests.
- `doctor --json` completes within a bounded timeout when configured ports are
  unreachable.
- Empty app dir reports missing token and missing extension copy.
- Empty app dir reports `RUN_INSTALL_BROWSER` and `LOAD_UNPACKED_EXTENSION` for
  missing extension prerequisites.
- Empty app dir may report `START_BRIDGE` for missing token readiness, but must
  not imply the bridge can become ready before extension install prerequisites
  are fixed.
- Missing extension version reports `RUN_INSTALL_BROWSER`, plus
  `LOAD_UNPACKED_EXTENSION`.
- Version mismatch reports expected and installed extension versions.
- No running bridge reports `START_BRIDGE`.
- No running bridge reports top-level `degraded` when install prerequisites are
  otherwise coherent.
- Non-bridge HTTP port conflict reports `STOP_CONFLICTING_PROCESS` and
  `USE_DIFFERENT_PORT`, and top-level `fail`.
- Running compatible bridge with disconnected extension reports
  `RELOAD_EXTENSION` and `LOAD_UNPACKED_EXTENSION`.
- Running compatible bridge with connected extension reports top-level `ok`.
- JSON output includes stable `status`, `summary`, `checks`, and `recovery`.
- `doctor --json` exits `0` when diagnostics complete, including degraded and
  fail reports.
- JSON output includes `schema_version: 1`.
- Every check includes `id`, `status`, `message`, `required`, `kind`, and
  `recovery`.
- Check ids come from the initial stable check id list unless the schema version
  is intentionally changed.
- In schema v1, every `kind:"prerequisite"` check has `required:true`, and every
  `kind:"readiness"` or `kind:"advisory"` check has `required:false`.
- JSON output never includes a full bearer token.
- Human output never includes a full bearer token.
- `status` output remains backward compatible.
- `/link` remains `404`.
- `cargo fmt --all --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo test` passes.
- `node --check` passes for extension JS and smoke scripts.
- `python3 scripts/validate_skill.py skills/tmwd-cdp-bridge` passes.
- `node scripts/smoke_no_extension.mjs` passes.
- `node scripts/skill_minimal_flow.mjs` passes after Skill updates.
- Real Edge smoke passes before release.
- Real Chrome smoke passes before release when Chrome is available.

## Suggested Slice Plan

Slice 1: Diagnostic model and CLI shell

- Add `doctor --json` with runtime/app-dir checks.
- Add status aggregation and recovery action deduplication tests.

Slice 2: Port, health, and extension checks

- Add HTTP `/health` probing.
- Add connected/disconnected extension classification.
- Add integration tests with real bridge process and tiny HTTP conflict server.

Slice 3: Skill and docs adoption

- Update Skill first-use flow to run `doctor --json`.
- Update README and troubleshooting.
- Extend `skill_minimal_flow` if needed.

Slice 4: Release hardening

- Run full validation bundle.
- Add release notes.
- Publish patch/minor release depending on CLI surface stability.

## Implementation Validation Record

Implemented in this slice:

- Added `src/doctor.rs` with schema v1 report/check/recovery models.
- Added `tmwd-cdp-bridge doctor` and `tmwd-cdp-bridge doctor --json`.
- Added read-only app-dir, token, extension-copy, version, pid, port, health,
  server identity/version, extension id/origin, extension connection, and
  advisory browser-detection checks.
- Updated README, Chinese README, troubleshooting, development docs, Skill
  guidance, and `scripts/skill_minimal_flow.mjs` to prefer `doctor --json`.
- Added unit tests for aggregation/recovery ordering.
- Added integration tests for empty app dir, JSON schema fields, coherent
  install without running bridge, non-bridge HTTP conflict, human output token
  secrecy, running disconnected bridge, and running connected bridge.

Validated locally:

- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `node --check extension/background.js`
- `node --check scripts/e2e_chrome_smoke.mjs`
- `node --check scripts/e2e_browser_inspect_url.mjs`
- `node --check scripts/smoke_no_extension.mjs`
- `node --check scripts/skill_minimal_flow.mjs`
- `node --check scripts/soak_bridge.mjs`
- `node --check scripts/smoke_real_browser.mjs`
- `python3 scripts/validate_skill.py skills/tmwd-cdp-bridge`
- `cargo test --test bridge_integration`
- `node scripts/smoke_no_extension.mjs`
- `node scripts/skill_minimal_flow.mjs`
- `node scripts/smoke_real_browser.mjs` with Microsoft Edge at
  `/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge`
- `BROWSER_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" node scripts/smoke_real_browser.mjs`

Pending before release:

- Add release notes for the version that ships `doctor`.
