use gpui::{ClickEvent, Context, Div, div, prelude::*, px, rgb};

use crate::model::Dir;
use crate::serial::config::{BAUD_RATES, FlowOpt, ParityOpt};
use crate::serial::worker::{RxMsg, SerialConn};

use super::ui::{ACCENT, BORDER, CTRL, Dropdown, GREEN, GREEN_BG, MUTED, ORANGE, PANEL, RED, TEXT};
use super::{Mode, SerialApp};

impl SerialApp {
    pub(super) fn refresh_ports(&mut self) {
        let mut ports: Vec<String> = serialport::available_ports()
            .map(|v| v.into_iter().map(|p| p.port_name).collect())
            .unwrap_or_default();
        ports.sort_by_key(|p| {
            (
                p.trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse::<u32>()
                    .unwrap_or(u32::MAX),
                p.clone(),
            )
        });
        if !ports.contains(&self.settings.port_name) {
            self.settings.port_name = ports.first().cloned().unwrap_or_default();
        }
        self.ports = ports;
    }

    pub(super) fn drain(&mut self, cx: &mut Context<Self>) {
        let mut msgs = vec![];
        if let Some(conn) = &self.conn {
            while let Ok(m) = conn.rx.try_recv() {
                msgs.push(m);
            }
        }
        let had_msgs = !msgs.is_empty();
        let mut changed = false;
        for m in msgs {
            match m {
                RxMsg::Data(d) => {
                    changed = true;
                    self.rx_bytes += d.len() as u64;
                    if self.mode == Mode::Modbus {
                        changed |= self.handle_modbus_rx(&d);
                    } else {
                        self.terminal.feed(&d);
                        self.push_entry(Dir::Rx, d);
                    }
                }
                RxMsg::Error(e) => {
                    changed = true;
                    self.conn = None;
                    self.set_status(format!("{e}, 串口已关闭"), true);
                    self.stop_modbus("串口已关闭");
                }
            }
        }
        if self.mode == Mode::Modbus {
            changed |= self.modbus_tick(cx);
        }
        if had_msgs || changed {
            cx.notify();
        }
    }

    fn toggle_conn(&mut self, cx: &mut Context<Self>) {
        if self.conn.take().is_some() {
            self.stop_modbus("串口已关闭");
            self.set_status("串口已关闭", false);
        } else {
            match SerialConn::open(&self.settings) {
                Ok(c) => {
                    self.conn = Some(c);
                    let s = &self.settings;
                    let msg = format!("已打开 {} @ {}", s.port_name, s.baud_rate);
                    self.set_status(msg, false);
                }
                Err(e) => self.set_status(e.to_string(), true),
            }
        }
        self.save(cx);
        cx.notify();
    }

    pub(super) fn apply_custom_baud(&mut self, cx: &mut Context<Self>) {
        if self.conn.is_some() {
            self.set_status("请先关闭串口再修改参数", true);
        } else {
            let text = self.custom_baud.read(cx).text().trim().to_string();
            match text.parse::<u32>() {
                Ok(b) if b > 0 => {
                    self.settings.baud_rate = b;
                    self.save(cx);
                }
                _ => self.set_status("波特率必须是正整数", true),
            }
        }
        cx.notify();
    }

    pub(super) fn config_panel(&self, cx: &Context<Self>) -> Div {
        let connected = self.conn.is_some();
        let s = &self.settings;

        let port_label = if s.port_name.is_empty() {
            "无可用串口".to_string()
        } else {
            s.port_name.clone()
        };
        let port_row = div()
            .flex()
            .gap_1()
            .child(div().flex_1().child(self.dropdown(
                Dropdown::Port,
                port_label,
                self.ports.clone(),
                |this, i, _| this.settings.port_name = this.ports[i].clone(),
                cx,
            )))
            .child(
                Self::button("refresh", "刷新", false)
                    .when(connected, |d| d.opacity(0.5))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if this.conn.is_none() {
                            this.refresh_ports();
                            cx.notify();
                        }
                    })),
            );

        let parity_idx = ParityOpt::ALL
            .iter()
            .position(|p| *p == s.parity)
            .unwrap_or(0);
        let flow_idx = FlowOpt::ALL.iter().position(|p| *p == s.flow).unwrap_or(0);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(4.)).h(px(18.)).rounded_full().bg(rgb(ORANGE)))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("串口调试助手"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.))
                    .rounded_full()
                    .text_size(px(11.))
                    .bg(rgb(if connected { GREEN_BG } else { CTRL }))
                    .text_color(rgb(if connected { GREEN } else { MUTED }))
                    .child(div().size(px(6.)).rounded_full().bg(rgb(if connected {
                        GREEN
                    } else {
                        0x6b7280
                    })))
                    .child(if connected { "已连接" } else { "未连接" }),
            );

        let dtr_on_open = div()
            .id("dtr-on-open")
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(rgb(if connected { MUTED } else { TEXT }))
            .child(
                div()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(rgb(if s.dtr_on_open { ACCENT } else { MUTED }))
                    .bg(rgb(if s.dtr_on_open { ACCENT } else { CTRL }))
                    .text_color(rgb(0xffffff))
                    .when(s.dtr_on_open, |d| d.child("✓")),
            )
            .child("打开串口时启用 DTR")
            .when(connected, |d| d.opacity(0.6))
            .when(!connected, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if this.conn.is_none() {
                            this.settings.dtr_on_open = !this.settings.dtr_on_open;
                            this.save(cx);
                            cx.notify();
                        }
                    }))
            });

        let open_button = Self::button(
            "open-close",
            if connected {
                "关闭串口"
            } else {
                "打开串口"
            },
            true,
        )
        .h(px(34.))
        .bg(rgb(if connected { RED } else { ACCENT }))
        .border_color(rgb(if connected { RED } else { ACCENT }))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_conn(cx)));

        div()
            .w(px(260.))
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(header)
            .child(Self::card(
                "连接",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(Self::field("串口", port_row))
                    .child(dtr_on_open)
                    .child(open_button),
            ))
            .child(Self::card(
                "参数",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(Self::field(
                        "波特率",
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(self.dropdown(
                                Dropdown::Baud,
                                s.baud_rate.to_string(),
                                BAUD_RATES.iter().map(|b| b.to_string()).collect(),
                                |this, i, cx| {
                                    this.settings.baud_rate = BAUD_RATES[i];
                                    this.custom_baud.update(cx, |t, cx| t.set_text("", cx));
                                },
                                cx,
                            ))
                            .child(self.custom_baud.clone()),
                    ))
                    .child(Self::field(
                        "数据位",
                        self.dropdown(
                            Dropdown::DataBits,
                            s.data_bits.to_string(),
                            (5..=8).map(|n| n.to_string()).collect(),
                            |this, i, _| this.settings.data_bits = 5 + i as u8,
                            cx,
                        ),
                    ))
                    .child(Self::field(
                        "停止位",
                        self.dropdown(
                            Dropdown::StopBits,
                            s.stop_bits.to_string(),
                            vec!["1".into(), "2".into()],
                            |this, i, _| this.settings.stop_bits = 1 + i as u8,
                            cx,
                        ),
                    ))
                    .child(Self::field(
                        "校验位",
                        self.dropdown(
                            Dropdown::Parity,
                            ParityOpt::ALL[parity_idx].label().to_string(),
                            ParityOpt::ALL
                                .iter()
                                .map(|p| p.label().to_string())
                                .collect(),
                            |this, i, _| this.settings.parity = ParityOpt::ALL[i],
                            cx,
                        ),
                    ))
                    .child(Self::field(
                        "流控",
                        self.dropdown(
                            Dropdown::Flow,
                            FlowOpt::ALL[flow_idx].label().to_string(),
                            FlowOpt::ALL.iter().map(|p| p.label().to_string()).collect(),
                            |this, i, _| this.settings.flow = FlowOpt::ALL[i],
                            cx,
                        ),
                    )),
            ))
    }
}
