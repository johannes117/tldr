use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::editor::Editor;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub editor: Option<Editor>,
    pub port_range: Option<(u16, u16)>,
    pub auto_open: Option<bool>,
}

fn config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "tldr", "tldr")
        .ok_or_else(|| anyhow!("no config dir"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let p = config_path()?;
        if p.exists() {
            let s = std::fs::read_to_string(&p)?;
            return Ok(toml::from_str(&s)?);
        }
        Ok(Self::default())
    }

    pub fn save(&self) -> Result<PathBuf> {
        let p = config_path()?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let s = toml::to_string_pretty(self)?;
        std::fs::write(&p, s)?;
        Ok(p)
    }
}
