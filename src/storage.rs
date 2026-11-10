use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::DataMode;
use crate::serial::config::PortSettings;

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Suffix {
    pub name: String,
    /// Escape-syntax source text, e.g. `\r\n` or `\x0D\x0A`.
    pub text: String,
}

pub const BUILTIN_COUNT: usize = 4;

pub fn builtin_suffixes() -> Vec<Suffix> {
    vec![
        Suffix { name: "无".into(), text: String::new() },
        Suffix { name: "\\r".into(), text: "\\r".into() },
        Suffix { name: "\\n".into(), text: "\\n".into() },
        Suffix { name: "\\r\\n".into(), text: "\\r\\n".into() },
    ]
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub settings: PortSettings,
    pub custom_suffixes: Vec<Suffix>,
    pub suffix_idx: usize,
    pub tx_mode: DataMode,
    pub rx_mode: DataMode,
    pub show_ts: bool,
    pub auto_scroll: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            settings: PortSettings::default(),
            custom_suffixes: vec![],
            suffix_idx: 3,
            tx_mode: DataMode::Ascii,
            rx_mode: DataMode::Ascii,
            show_ts: true,
            auto_scroll: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("serial-debug").join("config.json"))
}

pub fn load() -> AppConfig {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &AppConfig) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(p, s);
    }
}
