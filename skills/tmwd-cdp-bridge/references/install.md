# Install And Platform Detection

Use this reference when `tmwd-cdp-bridge` is not on `PATH`, `doctor` is
unavailable, or the user asks how to install the skill and binary.

## Preferred Release Install From A Repo Checkout

Use the repository installer when the current workspace contains this repository
checkout. It detects OS/arch and downloads the matching GitHub Release asset.

macOS/Linux:

```sh
SKILL_DIR="${SKILL_DIR:-$HOME/.codex/skills}" sh scripts/install.sh
```

Windows PowerShell:

```powershell
$env:SKILL_DIR = if ($env:SKILL_DIR) { $env:SKILL_DIR } else { "$HOME\.codex\skills" }
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

If the current agent uses another skills directory, set `SKILL_DIR` to that
directory before running the installer.

If `scripts/install.sh` or `scripts/install.ps1` is not present, use the manual
release asset mapping below instead of inventing a local script path.

## Source Checkout Fallback

If already inside this source checkout, replace `tmwd-cdp-bridge` with
`cargo run --` for commands, or build the binary:

```sh
cargo build --release
```

Then use `target/release/tmwd-cdp-bridge` or copy it to a directory on `PATH`.

## Manual Platform Mapping

Use release assets from:

`https://github.com/koda-claw/tmwd-cdp-bridge/releases`

Known asset names:

- macOS arm64: `tmwd-cdp-bridge-macos-arm64.tar.gz`
- macOS x64: `tmwd-cdp-bridge-macos-x64.tar.gz`
- Linux x64: `tmwd-cdp-bridge-linux-x64.tar.gz`
- Windows x64: `tmwd-cdp-bridge-windows-x64.zip`

macOS/Linux platform detection:

```sh
os="$(uname -s)"
arch="$(uname -m)"
case "$os:$arch" in
  Darwin:arm64|Darwin:aarch64) asset="tmwd-cdp-bridge-macos-arm64.tar.gz" ;;
  Darwin:x86_64|Darwin:amd64) asset="tmwd-cdp-bridge-macos-x64.tar.gz" ;;
  Linux:x86_64|Linux:amd64) asset="tmwd-cdp-bridge-linux-x64.tar.gz" ;;
  *) echo "unsupported platform: $os/$arch" >&2; exit 1 ;;
esac
```

Windows supports the `tmwd-cdp-bridge-windows-x64.zip` release asset.

Install the extracted binary into a directory on `PATH`, then rerun:

```sh
tmwd-cdp-bridge version
tmwd-cdp-bridge doctor --json
```

## After Install

Refresh extension files for the user's target browser:

```sh
tmwd-cdp-bridge install edge
# or
tmwd-cdp-bridge install chrome
```

Load the printed `extension/` directory in `edge://extensions` or
`chrome://extensions` with Developer mode enabled. Use `tmwd-cdp-bridge repair
edge` or `tmwd-cdp-bridge repair chrome` to reprint browser-side instructions.

Validate readiness with:

```sh
tmwd-cdp-bridge doctor --json
```
