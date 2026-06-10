use std::{
    env, fs,
    io::{self, Cursor},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Serialize;
use tar::Archive;

const DEFAULT_REPO: &str = "koda-claw/tmwd-cdp-bridge";
const BIN_NAME: &str = "tmwd-cdp-bridge";

#[derive(Debug, Clone, Serialize)]
pub struct UpgradePlan {
    pub version: String,
    pub repo: String,
    pub archive: String,
    pub url: String,
    pub destination: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpgradeOutcome {
    pub version: String,
    pub archive: String,
    pub destination: PathBuf,
    pub source: String,
    pub pending_restart: bool,
}

pub async fn upgrade_current_binary(
    version: Option<&str>,
    repo: Option<&str>,
) -> Result<UpgradeOutcome> {
    let plan = plan_upgrade(version, repo)?;
    let bytes = read_archive_bytes(&plan).await?;
    install_from_archive_bytes(&bytes, &plan)
}

async fn read_archive_bytes(plan: &UpgradePlan) -> Result<Vec<u8>> {
    if let Some(path) = plan.url.strip_prefix("file://") {
        return fs::read(path).with_context(|| format!("read {}", plan.url));
    }
    Ok(reqwest::get(&plan.url)
        .await
        .with_context(|| format!("download {}", plan.url))?
        .error_for_status()
        .with_context(|| format!("download {}", plan.url))?
        .bytes()
        .await
        .with_context(|| format!("read {}", plan.url))?
        .to_vec())
}

pub fn plan_upgrade(version: Option<&str>, repo: Option<&str>) -> Result<UpgradePlan> {
    let version = version
        .map(ToOwned::to_owned)
        .or_else(|| env::var("TMWD_CDP_BRIDGE_VERSION").ok())
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    let repo = repo
        .map(ToOwned::to_owned)
        .or_else(|| env::var("TMWD_CDP_BRIDGE_REPO").ok())
        .unwrap_or_else(|| DEFAULT_REPO.to_string());
    let archive = platform_archive()?;
    let url = env::var("TMWD_CDP_BRIDGE_UPGRADE_URL").unwrap_or_else(|_| {
        format!("https://github.com/{repo}/releases/download/{version}/{archive}")
    });
    let destination = current_binary_path()?;
    Ok(UpgradePlan {
        version,
        repo,
        archive,
        url,
        destination,
    })
}

pub fn install_from_archive_bytes(bytes: &[u8], plan: &UpgradePlan) -> Result<UpgradeOutcome> {
    let bin = extract_binary(bytes, &plan.archive)?;
    let pending_restart = replace_binary(&plan.destination, &bin)?;
    Ok(UpgradeOutcome {
        version: plan.version.clone(),
        archive: plan.archive.clone(),
        destination: plan.destination.clone(),
        source: plan.url.clone(),
        pending_restart,
    })
}

fn current_binary_path() -> Result<PathBuf> {
    env::current_exe().context("resolve current executable path")
}

fn platform_archive() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Ok("tmwd-cdp-bridge-macos-arm64.tar.gz".to_string()),
        ("macos", "x86_64") => Ok("tmwd-cdp-bridge-macos-x64.tar.gz".to_string()),
        ("linux", "x86_64") => Ok("tmwd-cdp-bridge-linux-x64.tar.gz".to_string()),
        ("windows", "x86_64") => Ok("tmwd-cdp-bridge-windows-x64.zip".to_string()),
        _ => {
            bail!("unsupported platform: {os} {arch}; build from source with cargo build --release")
        }
    }
}

fn extract_binary(bytes: &[u8], archive_name: &str) -> Result<Vec<u8>> {
    if archive_name.ends_with(".zip") {
        extract_binary_from_zip(bytes)
    } else {
        extract_binary_from_tar_gz(bytes)
    }
}

fn extract_binary_from_tar_gz(bytes: &[u8]) -> Result<Vec<u8>> {
    let gz = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(gz);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        if path_file_name(entry.path()?.as_ref()) == Some(BIN_NAME) {
            let mut out = Vec::new();
            io::copy(&mut entry, &mut out).context("read binary from tar archive")?;
            return Ok(out);
        }
    }
    bail!("archive did not contain {BIN_NAME}")
}

fn extract_binary_from_zip(bytes: &[u8]) -> Result<Vec<u8>> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("read zip archive")?;
    let expected = format!("{BIN_NAME}.exe");
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("read zip entry")?;
        let name = file.enclosed_name();
        if name.as_deref().and_then(path_file_name) == Some(expected.as_str()) {
            let mut out = Vec::new();
            io::copy(&mut file, &mut out).context("read binary from zip archive")?;
            return Ok(out);
        }
    }
    bail!("archive did not contain {expected}")
}

fn path_file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

fn replace_binary(destination: &Path, bytes: &[u8]) -> Result<bool> {
    let parent = destination
        .parent()
        .with_context(|| format!("resolve parent directory for {}", destination.display()))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .context("resolve executable file name")?;
    let tmp = parent.join(format!(".{file_name}.upgrade-tmp"));
    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    set_executable(&tmp)?;
    replace_binary_file(&tmp, destination, parent, file_name)
}

#[cfg(not(windows))]
fn replace_binary_file(
    tmp: &Path,
    destination: &Path,
    _parent: &Path,
    _file_name: &str,
) -> Result<bool> {
    fs::rename(tmp, destination)
        .inspect_err(|_| {
            let _ = fs::remove_file(tmp);
        })
        .with_context(|| format!("replace {}", destination.display()))?;
    Ok(false)
}

#[cfg(windows)]
fn replace_binary_file(
    tmp: &Path,
    destination: &Path,
    parent: &Path,
    file_name: &str,
) -> Result<bool> {
    use std::process::{self, Stdio};

    let script = parent.join(format!(".{file_name}.upgrade.cmd"));
    let tmp_s = tmp.display().to_string();
    let destination_s = destination.display().to_string();
    let script_s = script.display().to_string();
    let body = format!(
        "@echo off\r\n\
         setlocal\r\n\
         set PID={pid}\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq %PID%\" | find \"%PID%\" >NUL\r\n\
         if not errorlevel 1 (\r\n\
         timeout /T 1 /NOBREAK >NUL\r\n\
         goto wait\r\n\
         )\r\n\
         move /Y \"{tmp_s}\" \"{destination_s}\" >NUL\r\n\
         del \"{script_s}\" >NUL 2>NUL\r\n",
        pid = process::id()
    );
    fs::write(&script, body).with_context(|| format!("write {}", script.display()))?;
    process::Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("")
        .arg("/B")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("schedule replacement with {}", script.display()))?;
    Ok(true)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("chmod 0755 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    use tar::{Builder, Header};

    #[test]
    fn extracts_binary_from_unix_release_archive() {
        let bytes = tar_gz_with_file("./tmwd-cdp-bridge", b"new-binary");
        assert_eq!(
            extract_binary(&bytes, "tmwd-cdp-bridge-linux-x64.tar.gz").unwrap(),
            b"new-binary"
        );
    }

    #[test]
    fn extracts_binary_from_windows_release_archive() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("tmwd-cdp-bridge.exe", options).unwrap();
        writer.write_all(b"new-binary").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert_eq!(
            extract_binary(&bytes, "tmwd-cdp-bridge-windows-x64.zip").unwrap(),
            b"new-binary"
        );
    }

    #[test]
    fn archive_without_binary_is_rejected() {
        let bytes = tar_gz_with_file("./README.md", b"hello");
        assert!(
            extract_binary(&bytes, "tmwd-cdp-bridge-linux-x64.tar.gz")
                .unwrap_err()
                .to_string()
                .contains("did not contain")
        );
    }

    fn tar_gz_with_file(path: &str, content: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);
            let mut header = Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, content).unwrap();
            builder.finish().unwrap();
        }
        encoder.finish().unwrap()
    }
}
