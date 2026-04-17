use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub editor: Option<String>,
    pub port_range: Option<(u16, u16)>,
    pub auto_open: Option<bool>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("dev", "tldr", "tldr");
        let path = dirs.as_ref().map(|d| d.config_dir().join("config.toml"));
        if let Some(p) = path {
            if p.exists() {
                let s = std::fs::read_to_string(p)?;
                return Ok(toml::from_str(&s)?);
            }
        }
        Ok(Self::default())
    }
}
