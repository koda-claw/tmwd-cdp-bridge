use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

pub const APP_NAME: &str = "tmwd-cdp-bridge";
pub const DEFAULT_WS_PORT: u16 = 18765;
pub const DEFAULT_HTTP_PORT: u16 = 18766;
pub const EXTENSION_VERSION: &str = "2.1";
pub const ALLOWED_EXTENSION_ID: &str = "eghifjkffmcmffejmaaeicejpfopplem";
pub const ALLOWED_EXTENSION_ORIGIN: &str = "chrome-extension://eghifjkffmcmffejmaaeicejpfopplem";

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub app_dir: PathBuf,
    pub ws_port: u16,
    pub http_port: u16,
    pub allowed_extension_origin: String,
}

impl BridgeConfig {
    pub fn from_env() -> Result<Self> {
        let app_dir = env::var_os("CDP_BRIDGE_APP_DIR")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(default_app_dir)?;
        let ws_port = env_port("CDP_BRIDGE_WS_PORT", DEFAULT_WS_PORT)?;
        let http_port = env_port("CDP_BRIDGE_HTTP_PORT", DEFAULT_HTTP_PORT)?;
        let allowed_extension_origin = env::var("CDP_BRIDGE_ALLOWED_EXTENSION_ORIGIN")
            .unwrap_or_else(|_| ALLOWED_EXTENSION_ORIGIN.to_string());
        Ok(Self {
            app_dir,
            ws_port,
            http_port,
            allowed_extension_origin,
        })
    }

    pub fn ensure_app_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.app_dir)
            .with_context(|| format!("create app dir {}", self.app_dir.display()))?;
        set_private_dir_permissions(&self.app_dir)?;
        Ok(())
    }

    pub fn extension_dir(&self) -> PathBuf {
        self.app_dir.join("extension")
    }

    pub fn token_path(&self) -> PathBuf {
        self.app_dir.join("token")
    }

    pub fn version_path(&self) -> PathBuf {
        self.app_dir.join("version")
    }

    pub fn pid_path(&self) -> PathBuf {
        self.app_dir.join("pid")
    }

    pub fn installed_extension_version(&self) -> Result<Option<String>> {
        match fs::read_to_string(self.version_path()) {
            Ok(version) => {
                let version = version.trim().to_string();
                Ok((!version.is_empty()).then_some(version))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| {
                format!(
                    "read installed extension version {}",
                    self.version_path().display()
                )
            }),
        }
    }

    pub fn validate_installed_extension_version(&self) -> Result<()> {
        match self.installed_extension_version()? {
            Some(version) if version == EXTENSION_VERSION => Ok(()),
            Some(version) => anyhow::bail!(
                "extension version mismatch: installed {version}, expected {EXTENSION_VERSION}. Run 'tmwd-cdp-bridge install edge' or 'tmwd-cdp-bridge install chrome', then reload the browser extension."
            ),
            None => anyhow::bail!(
                "extension version file missing at {}. Run 'tmwd-cdp-bridge install edge' or 'tmwd-cdp-bridge install chrome', then load the printed extension directory.",
                self.version_path().display()
            ),
        }
    }
}

fn env_port(name: &str, default: u16) -> Result<u16> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u16>()
            .with_context(|| format!("{name} must be a TCP port")),
        Err(_) => Ok(default),
    }
}

fn default_app_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|dir| dir.join(APP_NAME))
        .context("could not determine platform data directory")
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ports_are_documented_ports() {
        let cfg = BridgeConfig {
            app_dir: PathBuf::from("x"),
            ws_port: DEFAULT_WS_PORT,
            http_port: DEFAULT_HTTP_PORT,
            allowed_extension_origin: ALLOWED_EXTENSION_ORIGIN.to_string(),
        };
        assert_eq!(cfg.ws_port, 18765);
        assert_eq!(cfg.http_port, 18766);
    }

    #[test]
    fn path_helpers_use_app_dir() {
        let cfg = BridgeConfig {
            app_dir: PathBuf::from("/tmp/tmwd-test"),
            ws_port: 1,
            http_port: 2,
            allowed_extension_origin: ALLOWED_EXTENSION_ORIGIN.to_string(),
        };
        assert_eq!(
            cfg.extension_dir(),
            PathBuf::from("/tmp/tmwd-test/extension")
        );
        assert_eq!(cfg.token_path(), PathBuf::from("/tmp/tmwd-test/token"));
        assert_eq!(cfg.version_path(), PathBuf::from("/tmp/tmwd-test/version"));
        assert_eq!(cfg.pid_path(), PathBuf::from("/tmp/tmwd-test/pid"));
    }

    #[test]
    fn installed_extension_version_reads_trimmed_value() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = BridgeConfig {
            app_dir: dir.path().to_path_buf(),
            ws_port: 1,
            http_port: 2,
            allowed_extension_origin: ALLOWED_EXTENSION_ORIGIN.to_string(),
        };
        assert_eq!(cfg.installed_extension_version().unwrap(), None);
        fs::write(cfg.version_path(), "2.1\n").unwrap();
        assert_eq!(
            cfg.installed_extension_version().unwrap(),
            Some("2.1".to_string())
        );
    }

    #[test]
    fn validate_installed_extension_version_rejects_missing_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = BridgeConfig {
            app_dir: dir.path().to_path_buf(),
            ws_port: 1,
            http_port: 2,
            allowed_extension_origin: ALLOWED_EXTENSION_ORIGIN.to_string(),
        };
        assert!(
            cfg.validate_installed_extension_version()
                .unwrap_err()
                .to_string()
                .contains("version file missing")
        );
        fs::write(cfg.version_path(), "1.0").unwrap();
        assert!(
            cfg.validate_installed_extension_version()
                .unwrap_err()
                .to_string()
                .contains("version mismatch")
        );
        fs::write(cfg.version_path(), EXTENSION_VERSION).unwrap();
        cfg.validate_installed_extension_version().unwrap();
    }
}
