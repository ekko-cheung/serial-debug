use std::ops::Range;

use chrono::Local;
use gpui::{Context, Div, ScrollStrategy, Window, div, prelude::*, px, rgb, uniform_list};

use crate::model::{DataMode, Dir, LogEntry, Row, rows_for};

use super::ui::{BLUE, BLUE_BG, BORDER, GREEN, GREEN_BG, TEXT};
use super::{Mode, SerialApp};

const MAX_ENTRIES: usize = 50_000;

impl SerialApp {
    fn mode_for(&self, dir: Dir) -> DataMode {
        if self.mode == Mode::Modbus {
            // Modbus frames are always shown as HEX, regardless of the debug modes.
            return DataMode::Hex;
        }
        match dir {
            Dir::Rx => self.rx_mode,
            Dir::Tx => self.tx_mode,
        }
    }

    pub(super) fn rebuild_rows(&mut self) {
        let rows: Vec<Row> = self
            .entries
            .iter()
            .flat_map(|e| rows_for(e, self.mode_for(e.dir)))
            .collect();
        self.rows = rows;
        self.scroll_to_end();
    }

    pub(super) fn scroll_to_end(&self) {
        if !self.auto_scroll {
            return;
        }
        match self.mode {
            Mode::Terminal => {
                let n = self.terminal.line_count();
                if n > 0 {
                    self.list_scroll
                        .scroll_to_item(n - 1, ScrollStrategy::Bottom);
                }
            }
            Mode::Debug => {
                if !self.rows.is_empty() {
                    self.list_scroll
                        .scroll_to_item(self.rows.len() - 1, ScrollStrategy::Bottom);
                }
            }
            Mode::Modbus => {
                if !self.rows.is_empty() {
                    self.list_scroll
                        .scroll_to_item(self.rows.len() - 1, ScrollStrategy::Bottom);
                }
                self.modbus.scroll_to_end();
            }
        }
    }

    pub(super) fn push_entry(&mut self, dir: Dir, data: Vec<u8>) {
        let entry = LogEntry {
            time: Local::now(),
            dir,
            data,
        };
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.drain(..MAX_ENTRIES / 10);
            self.entries.push(entry);
            self.rebuild_rows();
            return;
        }
        let mode = self.mode_for(dir);
        self.rows.extend(rows_for(&entry, mode));
        self.entries.push(entry);
        self.scroll_to_end();
    }

    pub(super) fn set_rx(&mut self, mode: DataMode, cx: &mut Context<Self>) {
        if self.rx_mode != mode {
            self.rx_mode = mode;
            self.rebuild_rows();
            self.save(cx);
        }
        cx.notify();
    }

    pub(super) fn set_tx(&mut self, mode: DataMode, cx: &mut Context<Self>) {
        if self.tx_mode != mode {
            self.tx_mode = mode;
            self.rebuild_rows();
            self.save(cx);
        }
        cx.notify();
    }

    pub(super) fn log_view(&self, cx: &mut Context<Self>) -> Div {
        let empty = self.rows.is_empty();
        div()
            .flex_1()
            .min_h_0()
            .m_3()
            .relative()
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x101216))
            .font_family("Consolas")
            .text_size(px(13.))
            .when(empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x4b5260))
                        .child("暂无数据"),
                )
            })
            .child(
                uniform_list(
                    "log",
                    self.rows.len(),
                    cx.processor(|this, range: Range<usize>, _w: &mut Window, _cx| {
                        range
                            .map(|ix| {
                                let r = &this.rows[ix];
                                let (tag, color, tag_bg) = match r.dir {
                                    Dir::Rx => ("RX", GREEN, GREEN_BG),
                                    Dir::Tx => ("TX", BLUE, BLUE_BG),
                                };
                                let first = r.time.is_some();
                                div()
                                    .h(px(22.))
                                    .px_3()
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .whitespace_nowrap()
                                    .when(ix % 2 == 1, |d| d.bg(rgb(0x14171c)))
                                    .when(this.show_ts, |d| {
                                        d.child(
                                            div()
                                                .w(px(88.))
                                                .flex_none()
                                                .text_size(px(12.))
                                                .text_color(rgb(0x6b7280))
                                                .child(r.time.clone().unwrap_or_default()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .h(px(16.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(4.))
                                            .text_size(px(10.))
                                            .when(first, |d| {
                                                d.bg(rgb(tag_bg)).text_color(rgb(color)).child(tag)
                                            }),
                                    )
                                    .child(div().text_color(rgb(TEXT)).child(r.text.clone()))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.list_scroll.clone())
                .size_full(),
            )
    }
}
