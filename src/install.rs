use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::config::{BridgeConfig, EXTENSION_VERSION};

const EXTENSION_FILES: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../extension/manifest.json")),
    ("background.js", include_str!("../extension/background.js")),
    ("content.js", include_str!("../extension/content.js")),
    ("config.js", include_str!("../extension/config.js")),
    (
        "disable_dialogs.js",
        include_str!("../extension/disable_dialogs.js"),
    ),
    ("popup.html", include_str!("../extension/popup.html")),
    ("popup.js", include_str!("../extension/popup.js")),
];

pub fn install_extension(config: &BridgeConfig, browser: &str) -> Result<String> {
    config.ensure_app_dir()?;
    let extension_dir = config.extension_dir();
    if extension_dir.exists() {
        fs::remove_dir_all(&extension_dir)
            .with_context(|| format!("remove {}", extension_dir.display()))?;
    }
    fs::create_dir_all(&extension_dir)
        .with_context(|| format!("create {}", extension_dir.display()))?;
    for (name, content) in EXTENSION_FILES {
        fs::write(extension_dir.join(name), content)
            .with_context(|| format!("write extension file {name}"))?;
    }
    fs::write(config.version_path(), EXTENSION_VERSION)?;
    Ok(install_instructions(browser, &extension_dir))
}

pub fn install_instructions(browser: &str, extension_dir: &Path) -> String {
    let url = match browser {
        "chrome" => "chrome://extensions",
        _ => "edge://extensions",
    };
    format!(
        "\
Extension copied to {path}

Manual load steps:
1. Open {url}
2. Enable Developer mode
3. Click Load unpacked
4. Select {path}
5. Confirm \"TMWD CDP Bridge\" is listed
",
        path = extension_dir.display()
    )
}
