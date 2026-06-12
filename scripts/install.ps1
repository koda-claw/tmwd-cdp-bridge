$ErrorActionPreference = "Stop"

$Version = if ($env:TMWD_CDP_BRIDGE_VERSION) { $env:TMWD_CDP_BRIDGE_VERSION } else { "v0.1.2" }
$Repo = if ($env:TMWD_CDP_BRIDGE_REPO) { $env:TMWD_CDP_BRIDGE_REPO } else { "koda-claw/tmwd-cdp-bridge" }
$BinDir = if ($env:BIN_DIR) { $env:BIN_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }
$SkillDir = if ($env:SKILL_DIR) { $env:SKILL_DIR } else { "" }

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "unsupported platform: Windows 64-bit is required"
}

$Archive = "tmwd-cdp-bridge-windows-x64.zip"
$Url = "https://github.com/$Repo/releases/download/$Version/$Archive"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("tmwd-cdp-bridge-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

try {
    $ZipPath = Join-Path $Tmp $Archive
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath
    Expand-Archive -Path $ZipPath -DestinationPath $Tmp -Force

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Copy-Item -Force (Join-Path $Tmp "tmwd-cdp-bridge.exe") (Join-Path $BinDir "tmwd-cdp-bridge.exe")
    Write-Host "Installed binary: $(Join-Path $BinDir "tmwd-cdp-bridge.exe")"

    if ($SkillDir) {
        New-Item -ItemType Directory -Force -Path $SkillDir | Out-Null
        $TargetSkill = Join-Path $SkillDir "tmwd-cdp-bridge"
        if (Test-Path $TargetSkill) {
            Remove-Item -Recurse -Force $TargetSkill
        }
        Copy-Item -Recurse -Force (Join-Path $Tmp "skills\tmwd-cdp-bridge") $TargetSkill
        Write-Host "Installed skill: $TargetSkill"
    } else {
        Write-Host "Skill not installed. Set SKILL_DIR, for example:"
        Write-Host '  $env:SKILL_DIR="$HOME\.codex\skills"; powershell -ExecutionPolicy Bypass -File scripts\install.ps1'
    }

    Write-Host "Next:"
    Write-Host "  tmwd-cdp-bridge install edge"
    Write-Host "  tmwd-cdp-bridge start"
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
