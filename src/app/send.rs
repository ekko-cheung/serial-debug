use gpui::{App, ClickEvent, Context, Div, div, prelude::*, px, rgb};

use crate::codec;
use crate::model::{DataMode, Dir};
use crate::storage::{self, BUILTIN_COUNT, SavedCommand, Suffix};

use super::SerialApp;
use super::ui::{BORDER, Dropdown, MUTED, PANEL, RED};

impl SerialApp {
    fn all_suffixes(&self) -> Vec<Suffix> {
        let mut v = storage::builtin_suffixes();
        v.extend(self.custom_suffixes.iter().cloned());
        v
    }

    pub(super) fn send(&mut self, cx: &mut Context<Self>) {
        let data = self.current_command(cx).encode();
        self.send_payload(data, cx);
    }

    pub(super) fn current_command(&self, cx: &App) -> SavedCommand {
        SavedCommand {
            text: self.send_input.read(cx).text().to_string(),
            mode: self.tx_mode,
            suffix: self.all_suffixes()[self.suffix_idx].text.clone(),
        }
    }

    pub(super) fn send_payload(&mut self, data: Result<Vec<u8>, String>, cx: &mut Context<Self>) {
        let data = match data {
            Ok(data) => data,
            Err(e) => {
                self.set_status(e, true);
                cx.notify();
                return;
            }
        };
        if data.is_empty() {
            return;
        }
        let res = match self.conn.as_mut() {
            Some(c) => c.write(&data),
            None => Err(anyhow::anyhow!("串口未打开")),
        };
        match res {
            Ok(()) => {
                self.tx_bytes += data.len() as u64;
                self.push_entry(Dir::Tx, data);
                self.status = None;
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
        cx.notify();
    }

    fn add_suffix(&mut self, cx: &mut Context<Self>) {
        let text = self.suffix_text.read(cx).text().to_string();
        let mut name = self.suffix_name.read(cx).text().trim().to_string();
        if let Err(e) = codec::parse_escapes(&text) {
            self.set_status(format!("后缀内容无效: {e}"), true);
        } else if text.is_empty() {
            self.set_status("后缀内容不能为空", true);
        } else {
            if name.is_empty() {
                name = text.clone();
            }
            self.custom_suffixes.push(Suffix { name, text });
            self.suffix_idx = BUILTIN_COUNT + self.custom_suffixes.len() - 1;
            self.show_add_suffix = false;
            self.suffix_name.update(cx, |i, cx| i.set_text("", cx));
            self.suffix_text.update(cx, |i, cx| i.set_text("", cx));
            self.status = None;
            self.save(cx);
        }
        cx.notify();
    }

    fn delete_suffix(&mut self, cx: &mut Context<Self>) {
        if self.suffix_idx >= BUILTIN_COUNT {
            self.custom_suffixes.remove(self.suffix_idx - BUILTIN_COUNT);
            self.suffix_idx = 3;
            self.save(cx);
            cx.notify();
        }
    }

    pub(super) fn send_panel(&self, cx: &Context<Self>) -> Div {
        let suffixes = self.all_suffixes();
        let cur = suffixes[self.suffix_idx].name.clone();
        let hex_err = if self.tx_mode == DataMode::Hex {
            codec::parse_hex(self.send_input.read(cx).text())
                .err()
                .map(|e| e.to_string())
        } else {
            None
        };

        let options_row =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(div().text_color(rgb(MUTED)).child("发送格式"))
                .child(self.seg(
                    "tx-mode",
                    ("ASCII", self.tx_mode == DataMode::Ascii),
                    ("HEX", self.tx_mode == DataMode::Hex),
                    |t, cx| t.set_tx(DataMode::Ascii, cx),
                    |t, cx| t.set_tx(DataMode::Hex, cx),
                    cx,
                ))
                .child(div().text_color(rgb(MUTED)).child("后缀"))
                .child(div().w(px(140.)).child(self.dropdown(
                    Dropdown::Suffix,
                    cur,
                    suffixes.iter().map(|s| s.name.clone()).collect(),
                    |this, i, cx| {
                        this.suffix_idx = i;
                        this.save(cx);
                        cx.notify();
                    },
                    cx,
                )))
                .child(
                    Self::button("add-suffix", "添加后缀", self.show_add_suffix).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.show_add_suffix = !this.show_add_suffix;
                            cx.notify();
                        }),
                    ),
                )
                .when(self.suffix_idx >= BUILTIN_COUNT, |d| {
                    d.child(Self::button("del-suffix", "删除当前后缀", false).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.delete_suffix(cx)),
                    ))
                })
                .when_some(hex_err, |d, e| {
                    d.child(div().text_size(px(12.)).text_color(rgb(RED)).child(e))
                });

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(options_row)
            .when(self.show_add_suffix, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(160.)).child(self.suffix_name.clone()))
                        .child(div().flex_1().child(self.suffix_text.clone()))
                        .child(Self::button("suffix-ok", "确定", true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.add_suffix(cx)),
                        ))
                        .child(
                            Self::button("suffix-cancel", "取消", false).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.show_add_suffix = false;
                                    cx.notify();
                                },
                            )),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(self.send_input.clone()))
                    .child(
                        Self::button("send", "发送", true)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(cx))),
                    ),
            )
    }
}
