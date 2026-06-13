# Doctor Recovery

Use this reference after running:

```sh
tmwd-cdp-bridge doctor --json
```

Branch on the JSON report, not the process exit code. `doctor --json` exits `0`
when diagnostics complete, including `degraded` and `fail` reports.

## Status Policy

- `ok`: proceed to `/health`, token read, and `/v1/rpc`.
- `degraded`: follow only listed local `recovery` actions, then rerun `doctor --json`.
- `fail`: do not proceed with browser page work. Resolve install/version/port prerequisites first.

## Recovery Actions

- `START_BRIDGE`: run `tmwd-cdp-bridge start`. Remember that you started it so you can stop it later.
- `RUN_INSTALL_EDGE`: run `tmwd-cdp-bridge install edge`.
- `RUN_INSTALL_CHROME`: run `tmwd-cdp-bridge install chrome`.
- `RUN_INSTALL_BROWSER`: choose the user's intended browser and run `install edge` or `install chrome`.
- `LOAD_UNPACKED_EXTENSION`: ask the user to load the printed extension directory in `edge://extensions` or `chrome://extensions`.
- `RELOAD_EXTENSION`: ask the user to reload the unpacked extension in the browser extensions page.
- `DISABLE_LEGACY_EXTENSION`: ask the user to disable old extension id `aikfggdiblmijobpgdapacebmcjknbof`; do not use the old protocol.
- `STOP_CONFLICTING_PROCESS`: ask before stopping anything; never kill unrelated processes automatically.
- `USE_DIFFERENT_PORT`: set both `CDP_BRIDGE_HTTP_PORT` and `CDP_BRIDGE_WS_PORT` to unused local ports.
- `UPGRADE_BINARY`: run `tmwd-cdp-bridge upgrade`, or install a newer release if `upgrade` is unavailable.
- `FIX_TOKEN_FILE`: start the bridge once to recreate the token, or fix file ownership/permissions. Never print the token.
- `REPAIR_INSTALL`: run `tmwd-cdp-bridge repair edge` or `tmwd-cdp-bridge repair chrome` to reprint instructions.

## Safe Recovery Order

1. Resolve `fail` prerequisites first: extension copy/version, server identity/version, port conflicts.
2. Start or reuse a bridge only when `START_BRIDGE` is listed and no non-bridge port conflict is reported.
3. Load/reload the browser extension after install/version checks are coherent.
4. Rerun `doctor --json` after each recovery step.
5. Continue to page work only after `/health` reports the expected server,
   fixed extension id, and `extension_connected:true`.

## Port Conflicts

If `STOP_CONFLICTING_PROCESS` appears, treat it as manual guidance. Prefer
custom ports when the conflicting process is unknown:

```sh
CDP_BRIDGE_WS_PORT=28765 CDP_BRIDGE_HTTP_PORT=28766 tmwd-cdp-bridge start
```

Keep both port variables together. Do not silently change only one side.
