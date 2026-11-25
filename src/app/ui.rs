use gpui::{
    ClickEvent, ClipboardItem, Context, Div, ElementId, MouseButton, SharedString, Stateful,
    Window, deferred, div, prelude::*, px, rgb,
};

use crate::model::{DataMode, export_text};

use super::terminal::EOL_LABELS;
use super::{Mode, SerialApp};

// Palette
const BG: u32 = 0x131519;
pub(super) const PANEL: u32 = 0x1a1c21;
const CARD: u32 = 0x1f2228;
pub(super) const CTRL: u32 = 0x282b33;
const CTRL_HOVER: u32 = 0x323641;
pub(super) const BORDER: u32 = 0x30343c;
pub(super) const TEXT: u32 = 0xe8eaed;
pub(super) const MUTED: u32 = 0x8b929c;
pub(super) const ACCENT: u32 = 0x4f8cff;
pub(super) const GREEN: u32 = 0x4ade80;
pub(super) const GREEN_BG: u32 = 0x173325;
pub(super) const BLUE: u32 = 0x6cb0ff;
pub(super) const BLUE_BG: u32 = 0x172a47;
pub(super) const RED: u32 = 0xf05252;
pub(super) const ORANGE: u32 = 0xf26a2e;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dropdown {
    Port,
    Baud,
    DataBits,
    StopBits,
    Parity,
    Flow,
    Suffix,
    ModbusFc,
    ModbusDecode,
    ModbusOrder,
}

impl Dropdown {
    fn id(self) -> &'static str {
        match self {
            Self::Port => "dd-port",
            Self::Baud => "dd-baud",
            Self::DataBits => "dd-data",
            Self::StopBits => "dd-stop",
            Self::Parity => "dd-parity",
            Self::Flow => "dd-flow",
            Self::Suffix => "dd-suffix",
            Self::ModbusFc => "dd-mb-fc",
            Self::ModbusDecode => "dd-mb-decode",
            Self::ModbusOrder => "dd-mb-order",
        }
    }

    fn is_port_setting(self) -> bool {
        matches!(
            self,
            Self::Port | Self::Baud | Self::DataBits | Self::StopBits | Self::Parity | Self::Flow
        )
    }
}

type PickFn = fn(&mut SerialApp, usize, &mut Context<SerialApp>);

impl SerialApp {
    pub(super) fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        active: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(if active { ACCENT } else { BORDER }))
            .bg(rgb(if active { ACCENT } else { CTRL }))
            .text_color(rgb(if active { 0xffffff } else { TEXT }))
            .cursor_pointer()
            .hover(move |s| s.bg(rgb(if active { 0x6aa0ff } else { CTRL_HOVER })))
            .child(label.into())
    }

    /// A card section with a small title.
    pub(super) fn card(title: &str, body: impl IntoElement) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(10.))
            .bg(rgb(CARD))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(title.to_string()),
            )
            .child(body)
    }

    pub(super) fn field(label: &str, child: impl IntoElement) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(label.to_string()),
            )
            .child(child)
    }

    pub(super) fn dropdown(
        &self,
        which: Dropdown,
        current: String,
        options: Vec<String>,
        pick: PickFn,
        cx: &Context<Self>,
    ) -> Div {
        let id = which.id();
        let port_setting = which.is_port_setting();
        let disabled = port_setting && self.conn.is_some();
        let open = self.open_dropdown == Some(which);
        let trigger = div()
            .id(id)
            .h(px(30.))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(if open { ACCENT } else { BORDER }))
            .bg(rgb(CTRL))
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| {
                d.cursor_pointer().hover(|s| s.bg(rgb(CTRL_HOVER)))
            })
            .child(div().truncate().child(current))
            .child(div().text_color(rgb(MUTED)).child("▾"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if disabled {
                    return;
                }
                this.open_dropdown = if this.open_dropdown == Some(which) {
                    None
                } else {
                    Some(which)
                };
                cx.notify();
            }));

        let menu = div()
            .id((id, 9999usize))
            .absolute()
            .when(port_setting, |d| d.top(px(34.)))
            .when(!port_setting, |d| d.bottom(px(34.)))
            .left_0()
            .w_full()
            .max_h(px(240.))
            .overflow_y_scroll()
            .occlude()
            .p_1()
            .bg(rgb(0x2d3039))
            .border_1()
            .border_color(rgb(0x444955))
            .rounded(px(8.))
            .shadow_lg()
            .children(options.into_iter().enumerate().map(|(i, label)| {
                div()
                    .id((id, i))
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(ACCENT)))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        pick(this, i, cx);
                        this.open_dropdown = None;
                        cx.notify();
                    }))
            }));

        div()
            .relative()
            .w_full()
            .child(trigger)
            .when(open, |d| d.child(deferred(menu).with_priority(2)))
    }

    pub(super) fn seg(
        &self,
        id: &'static str,
        a: (&'static str, bool),
        b: (&'static str, bool),
        on_a: fn(&mut Self, &mut Context<Self>),
        on_b: fn(&mut Self, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> Div {
        let item = |idx: usize, (label, active): (&'static str, bool)| {
            div()
                .id((id, idx))
                .h(px(26.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(active, |d| d.bg(rgb(ACCENT)).text_color(rgb(0xffffff)))
                .when(!active, |d| {
                    d.text_color(rgb(MUTED)).hover(|s| s.bg(rgb(CTRL_HOVER)))
                })
                .child(label)
        };
        div()
            .flex()
            .p(px(2.))
            .gap_1()
            .rounded(px(7.))
            .bg(rgb(CTRL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                item(0, a).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_a(this, cx))),
            )
            .child(
                item(1, b).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_b(this, cx))),
            )
    }

    fn toolbar(&self, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().text_color(rgb(MUTED)).child("模式"))
            .child(
                div()
                    .flex()
                    .p(px(2.))
                    .gap_1()
                    .rounded(px(7.))
                    .bg(rgb(CTRL))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .children(
                        [
                            ("mode-dbg", "调试", Mode::Debug),
                            ("mode-term", "终端", Mode::Terminal),
                            ("mode-modbus", "Modbus", Mode::Modbus),
                        ]
                        .map(|(id, label, mode)| {
                            let active = self.mode == mode;
                            div()
                                .id(id)
                                .h(px(26.))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(5.))
                                .cursor_pointer()
                                .text_size(px(12.))
                                .when(active, |d| d.bg(rgb(ACCENT)).text_color(rgb(0xffffff)))
                                .when(!active, |d| {
                                    d.text_color(rgb(MUTED)).hover(|s| s.bg(rgb(CTRL_HOVER)))
                                })
                                .child(label)
                                .on_click(cx.listener(move |this, _: &ClickEvent, w, cx| {
                                    this.set_mode(mode, w, cx)
                                }))
                        }),
                    ),
            )
            .when(self.mode == Mode::Debug, |d| {
                d.child(div().text_color(rgb(MUTED)).child("接收显示"))
                    .child(self.seg(
                        "rx-mode",
                        ("ASCII", self.rx_mode == DataMode::Ascii),
                        ("HEX", self.rx_mode == DataMode::Hex),
                        |t, cx| t.set_rx(DataMode::Ascii, cx),
                        |t, cx| t.set_rx(DataMode::Hex, cx),
                        cx,
                    ))
            })
            .when(self.mode != Mode::Terminal, |d| {
                d.child(
                    Self::button("ts", "时间戳", self.show_ts).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.show_ts = !this.show_ts;
                            this.save(cx);
                            cx.notify();
                        },
                    )),
                )
            })
            .when(self.mode == Mode::Terminal, |d| {
                d.child(
                    Self::button("echo", "本地回显", self.term_echo).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.term_echo = !this.term_echo;
                            cx.notify();
                        },
                    )),
                )
            })
            .when(self.mode == Mode::Terminal, |d| {
                d.child(
                    Self::button("eol", format!("回车: {}", EOL_LABELS[self.term_eol]), false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.term_eol = (this.term_eol + 1) % EOL_LABELS.len();
                            cx.notify();
                        })),
                )
                .child(
                    Self::button("lfcr", "LF 自动回车", self.terminal.lf_implies_cr).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.terminal.lf_implies_cr = !this.terminal.lf_implies_cr;
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(
                Self::button("autoscroll", "自动滚动", self.auto_scroll).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.auto_scroll = !this.auto_scroll;
                        this.scroll_to_end();
                        this.save(cx);
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::button("copy-all", "复制全部", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let text = match this.mode {
                            Mode::Terminal => this.terminal.text(),
                            Mode::Modbus => this.modbus_export(),
                            Mode::Debug => export_text(&this.entries, this.rx_mode, this.tx_mode),
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.set_status("日志已复制到剪贴板", false);
                        cx.notify();
                    },
                )),
            )
            .child(Self::button("clear", "清空", false).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.clear_modbus();
                    this.entries.clear();
                    this.rows.clear();
                    this.terminal.clear();
                    this.tx_bytes = 0;
                    this.rx_bytes = 0;
                    cx.notify();
                },
            )))
    }

    fn status_bar(&self) -> Div {
        let (msg, is_err) = self
            .status
            .clone()
            .unwrap_or_else(|| (String::new(), false));
        div()
            .flex_none()
            .h(px(24.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(0x0f1114))
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(if is_err { RED } else { MUTED }))
                    .when(!msg.is_empty(), |d| {
                        d.child(div().size(px(6.)).rounded_full().bg(rgb(if is_err {
                            RED
                        } else {
                            GREEN
                        })))
                    })
                    .child(msg),
            )
            .child(div().text_color(rgb(MUTED)).child(format!(
                "TX: {} B    RX: {} B",
                self.tx_bytes, self.rx_bytes
            )))
    }
}

impl Render for SerialApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config_panel(cx);
        let toolbar = self.toolbar(cx);
        let status = self.status_bar();

        let main = div().flex_1().min_w_0().flex().flex_col().child(toolbar);
        let main = match self.mode {
            Mode::Terminal => main.child(self.terminal_view(cx)),
            Mode::Debug => main.child(self.log_view(cx)).child(self.send_panel(cx)),
            Mode::Modbus => main
                .child(self.modbus_results_view(cx))
                .child(self.log_view(cx))
                .child(self.modbus_panel(cx)),
        };

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(13.))
            .font_family("Microsoft YaHei UI")
            .child(div().flex_1().min_h_0().flex().child(config).child(main))
            .child(status)
            .when(self.open_dropdown.is_some(), |d| {
                // Click-away backdrop below the open menu.
                d.child(
                    deferred(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .occlude()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.open_dropdown = None;
                                    cx.notify();
                                }),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }
}
