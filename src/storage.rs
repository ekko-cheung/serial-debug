use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::modbus::{ByteOrder, Decode};
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct ModbusConfig {
    pub slave: String,
    pub fc: u8,
    pub start: String,
    pub qty: String,
    pub value: String,
    pub values: String,
    pub poll_interval: String,
    pub coil_on: bool,
    pub decode: Decode,
    pub byte_order: ByteOrder,
}

impl Default for ModbusConfig {
    fn default() -> Self {
        Self {
            slave: "1".into(),
            fc: 3,
            start: "0".into(),
            qty: "1".into(),
            value: "0".into(),
            values: "0".into(),
            poll_interval: "1000".into(),
            coil_on: true,
            decode: Decode::Raw,
            byte_order: ByteOrder::Abcd,
        }
    }
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
    pub modbus: ModbusConfig,
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
            modbus: ModbusConfig::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_modbus_uses_defaults() {
        let json = r#"{
            "settings": {
                "port_name": "COM2",
                "baud_rate": 115200,
                "data_bits": 8,
                "stop_bits": 1,
                "parity": "None",
                "flow": "None"
            },
            "custom_suffixes": [],
            "suffix_idx": 3,
            "tx_mode": "Ascii",
            "rx_mode": "Ascii",
            "show_ts": true,
            "auto_scroll": true
        }"#;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.modbus, ModbusConfig::default());
    }

    #[test]
    fn modbus_config_round_trip() {
        let mut cfg = AppConfig::default();
        cfg.modbus.decode = Decode::F32;
        cfg.modbus.byte_order = ByteOrder::Cdab;
        cfg.modbus.fc = 0x10;
        cfg.modbus.values = "1 2 3".into();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.modbus, cfg.modbus);
    }
}
