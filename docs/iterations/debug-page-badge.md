# Optional Debug Page Badge Iteration Plan

## Goal

Add an optional page-level debug badge for humans who need to confirm that the
extension content script is present on a page. The production default remains
silent: normal browsing and Agent-controlled pages must not show injected UI.

This badge is a diagnostic affordance, not the source of truth for Agent
automation. Agents continue to rely on `tmwd-cdp-bridge status --json`,
`/health`, and authenticated `/v1/rpc`.

## Scope

- Add a popup Debug control that toggles page badge visibility.
- Store the toggle in `chrome.storage.local` so it survives extension reloads.
- Render the badge only in the top frame.
- Keep the badge small, non-modal, and visually distinct from page content.
- Make the badge status text precise:
  - `Injected` means the content script is present.
  - `Bridge connected` means the local bridge reports an active extension
    WebSocket.
  - `Bridge offline` means the content script is present but the local bridge is
    unavailable or not connected.
- Keep the default state off.
- Keep the current content-script request channel:
  `__tmwd_cdp_bridge_request`, with `__ljq_045ef1` as a legacy alias.

Out of scope:

- Any Agent protocol dependency on the badge.
- Always-on page UI.
- Browser Web Store packaging.
- Removing the legacy `__ljq_045ef1` alias.

## Proposed User Experience

The extension popup remains the primary status surface. It gets a Debug toggle
near the status or footer area:

- Off: pages show no badge.
- On: the active page shows a compact badge, for example `TMWD`.
- Hover or click expands a small diagnostic popover with tab URL, tab id,
  content-script state, bridge state, and last checked time.
- The popover includes a `Hide` action that turns the toggle off.

The badge should avoid `alert()`, large panels, animations that affect page
layout, and copy that implies Agent control when only injection is confirmed.

## Implementation Steps

1. Define storage contract
   - Files: `extension/background.js`, `extension/popup.js`,
     `extension/content.js`
   - Add a stable key such as `tmwdShowPageBadge`.
   - Add a small background message that returns bridge health using the same
     configured health URL as the popup.
   - Verify: `node --check extension/background.js extension/popup.js
     extension/content.js`.

2. Add popup toggle
   - Files: `extension/popup.html`, `extension/popup.js`
   - Add a compact Debug toggle without crowding the existing status panels.
   - Persist the toggle to `chrome.storage.local`.
   - Notify the active tab after changes so the badge appears or disappears
     without a full page reload when possible.
   - Verify: static popup screenshot, no text overflow at the existing popup
     width, and `node --check extension/popup.js`.

3. Add optional badge renderer
   - Files: `extension/content.js`
   - Render only when `window.self === window.top` and the stored toggle is on.
   - Do not render on extension-internal, browser-internal, or unsupported pages.
   - Keep the badge fixed, small, and removable. It must not shift page layout.
   - Use precise text: `Injected`, `Bridge connected`, or `Bridge offline`.
   - Verify: real browser smoke plus manual visual check on a normal HTTP page.

4. Preserve request-channel compatibility
   - Files: `extension/config.js`, `extension/content.js`
   - Keep `__tmwd_cdp_bridge_request` as the preferred request node id.
   - Keep `__ljq_045ef1` as a legacy alias.
   - Verify: add or extend browser smoke to exercise the preferred id and the
     legacy id.

5. Version and install guidance
   - Files: `extension/manifest.json`, `src/config.rs`, tests as needed
   - Bump the extension version when the badge implementation lands.
   - Ensure `install edge` / `install chrome` writes the matching version file.
   - Verify: `cargo test`, `node scripts/smoke_no_extension.mjs`,
     `node scripts/skill_minimal_flow.mjs`.

6. Real browser validation
   - Files: `scripts/smoke_real_browser.mjs` if automation coverage is extended.
   - Run Edge smoke and Chrome smoke when available.
   - Confirm extension reload/reconnect still works and no page badge appears by
     default.
   - Verify: `node scripts/smoke_real_browser.mjs`; optionally run with
     `BROWSER_BIN` pointing to Chrome.

## Risks And Mitigations

| Risk | Mitigation |
| --- | --- |
| Badge obscures website controls | Default off; render as a compact fixed badge; include Hide; avoid layout changes. |
| Badge misleads users about Agent readiness | Use precise status labels and keep `status --json` as the authoritative Agent signal. |
| Badge affects Agent screenshots or DOM reasoning | Default off; document that Agents should not enable it for real tasks unless debugging. |
| Storage state behaves differently across Edge and Chrome | Use MV3 `chrome.storage.local` only; validate Edge and Chrome reload paths. |
| Content script and popup disagree on custom bridge ports | Reuse the existing configured health URL path instead of hardcoding defaults. |
| Existing scripts use the old request node id | Preserve `__ljq_045ef1` as a legacy alias and add compatibility smoke coverage. |
| Extension reload races return | Keep current local WebSocket socket handling; include reload/reconnect smoke. |

## Acceptance Criteria

- Default install/reload shows no page badge on normal websites.
- Popup exposes a Debug toggle for page badge visibility.
- Enabling the toggle shows a compact top-frame-only badge on HTTP/HTTPS pages.
- Disabling the toggle removes the badge without requiring a browser restart.
- Badge copy distinguishes content-script injection from bridge connection.
- Badge never uses `ljq_driver` naming.
- Preferred request id `__tmwd_cdp_bridge_request` works.
- Legacy request id `__ljq_045ef1` still works.
- `cargo fmt --all --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo test` passes.
- `node --check` passes for extension JS and smoke scripts.
- `python3 scripts/validate_skill.py skills/tmwd-cdp-bridge` passes.
- `node scripts/smoke_no_extension.mjs` passes.
- `node scripts/skill_minimal_flow.mjs` passes.
- `node scripts/smoke_real_browser.mjs` passes on Edge.
- Chrome real browser smoke passes when Chrome is available. Chrome 137+ should
  be loaded through DevTools `Extensions.loadUnpacked`, not command-line
  `--load-extension`.

## Release Notes Checklist

- Mention that page badge is a Debug-only optional diagnostic.
- Mention that default page behavior remains silent.
- Mention the extension version bump and require browser extension reload.
- Mention that `__ljq_045ef1` remains only as a compatibility alias.

## Implementation Validation Record

Current implementation result:

- Edge real browser smoke passed with default badge hidden, badge visible after
  enabling `tmwdShowPageBadge`, preferred request id working, legacy request id
  working, and extension reload/reconnect still healthy.
- Chrome real browser smoke passed with
  `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` after switching
  the harness from command-line extension loading to DevTools
  `Extensions.loadUnpacked`; the fixed id
  `eghifjkffmcmffejmaaeicejpfopplem` was preserved and accepted by the bridge.
