use std::{env, fs, path::Path};

use anyhow::{Context, Result};
use rand::RngCore;

pub fn load_or_create_token(path: &Path) -> Result<String> {
    if let Ok(token) = env::var("CDP_BRIDGE_TOKEN") {
        write_token(path, &token)?;
        return Ok(token);
    }
    if let Ok(token) = fs::read_to_string(path) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    write_token(path, &token)?;
    Ok(token)
}

pub fn write_token(path: &Path, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create token parent {}", parent.display()))?;
    }
    write_private(path, token.as_bytes())?;
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open token {}", path.display()))?;
    use std::io::Write;
    file.write_all(data)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).with_context(|| format!("write token {}", path.display()))?;
    Ok(())
}

pub fn token_prefix(token: &str) -> String {
    let prefix: String = token.chars().take(8).collect();
    format!("{prefix}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let first = load_or_create_token(&path).unwrap();
        let second = load_or_create_token(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
    }
}
