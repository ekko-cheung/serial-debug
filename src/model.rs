use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::codec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Rx,
    Tx,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum DataMode {
    Ascii,
    Hex,
}

/// Raw bytes are stored so the display mode can be switched at any time.
pub struct LogEntry {
    pub time: DateTime<Local>,
    pub dir: Dir,
    pub data: Vec<u8>,
}

/// One visual line of the log (an entry may span several lines).
pub struct Row {
    pub dir: Dir,
    pub time: Option<String>,
    pub text: String,
}

const HEX_BYTES_PER_ROW: usize = 24;
const ASCII_CHARS_PER_ROW: usize = 80;

pub fn rows_for(entry: &LogEntry, mode: DataMode) -> Vec<Row> {
    let chunks: Vec<String> = match mode {
        DataMode::Hex => entry
            .data
            .chunks(HEX_BYTES_PER_ROW)
            .map(codec::format_hex)
            .collect(),
        DataMode::Ascii => {
            let s: Vec<char> = codec::format_ascii(&entry.data).chars().collect();
            if s.is_empty() {
                vec![String::new()]
            } else {
                s.chunks(ASCII_CHARS_PER_ROW)
                    .map(|c| c.iter().collect())
                    .collect()
            }
        }
    };
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, text)| Row {
            dir: entry.dir,
            time: (i == 0).then(|| entry.time.format("%H:%M:%S%.3f").to_string()),
            text,
        })
        .collect()
}

/// Plain-text rendering of the whole log (for "copy all").
pub fn export_text(entries: &[LogEntry], rx_mode: DataMode, tx_mode: DataMode) -> String {
    let mut out = String::new();
    for e in entries {
        let mode = if e.dir == Dir::Rx { rx_mode } else { tx_mode };
        let body = match mode {
            DataMode::Hex => codec::format_hex(&e.data),
            DataMode::Ascii => codec::format_ascii(&e.data),
        };
        let d = if e.dir == Dir::Tx { "TX" } else { "RX" };
        out.push_str(&format!("[{}] {} {}\n", e.time.format("%H:%M:%S%.3f"), d, body));
    }
    out
}
