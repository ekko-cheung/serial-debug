use std::ops::Range;
use std::time::Duration;

use chrono::Local;
use gpui::{
    deferred, div, prelude::*, px, rgb, uniform_list, ClickEvent, ClipboardItem, Context, Div,
    ElementId, Entity, MouseButton, ScrollStrategy, SharedString, Stateful, Subscription, Task,
    UniformListScrollHandle, Window,
};

use crate::codec;
use crate::model::{export_text, rows_for, DataMode, Dir, LogEntry, Row};
use crate::serial::config::{FlowOpt, ParityOpt, PortSettings, BAUD_RATES};
use crate::serial::worker::{RxMsg, SerialConn};
use crate::storage::{self, AppConfig, Suffix, BUILTIN_COUNT};
use crate::text_input::{Submitted, TextInput};

const MAX_ENTRIES: usize = 50_000;

// Palette
const BG: u32 = 0x131519;
const PANEL: u32 = 0x1a1c21;
const CARD: u32 = 0x1f2228;
const CTRL: u32 = 0x282b33;
const CTRL_HOVER: u32 = 0x323641;
const BORDER: u32 = 0x30343c;
const TEXT: u32 = 0xe8eaed;
const MUTED: u32 = 0x8b929c;
const ACCENT: u32 = 0x4f8cff;
const GREEN: u32 = 0x4ade80;
const GREEN_BG: u32 = 0x173325;
const BLUE: u32 = 0x6cb0ff;
const BLUE_BG: u32 = 0x172a47;
const RED: u32 = 0xf05252;
const ORANGE: u32 = 0xf26a2e;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dd {
    Port,
    Baud,
    DataBits,
    StopBits,
    Parity,
    Flow,
    Suffix,
}

type PickFn = fn(&mut SerialApp, usize, &mut Context<SerialApp>);

pub struct SerialApp {
    ports: Vec<String>,
    settings: PortSettings,
    custom_baud: Entity<TextInput>,
    conn: Option<SerialConn>,

    entries: Vec<LogEntry>,
    rows: Vec<Row>,
    rx_mode: DataMode,
    tx_mode: DataMode,
    show_ts: bool,
    auto_scroll: bool,

    custom_suffixes: Vec<Suffix>,
    suffix_idx: usize,
    show_add_suffix: bool,
    suffix_name: Entity<TextInput>,
    suffix_text: Entity<TextInput>,
    send_input: Entity<TextInput>,

    open_dd: Option<Dd>,
    status: Option<(String, bool)>,
    tx_bytes: u64,
    rx_bytes: u64,
    list_scroll: UniformListScrollHandle,

    _subs: Vec<Subscription>,
    _poll: Task<()>,
}

impl SerialApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cfg = storage::load();
        let custom_baud = cx.new(|cx| TextInput::new(cx, "自定义波特率, 回车应用"));
        let suffix_name = cx.new(|cx| TextInput::new(cx, "后缀名称"));
        let suffix_text = cx.new(|cx| TextInput::new(cx, "后缀内容, 如 \\r\\n 或 \\x0D\\x0A"));
        let send_input = cx.new(|cx| TextInput::new(cx, "输入要发送的内容, 回车发送"));

        let subs = vec![
            cx.subscribe(&custom_baud, |this, _, _: &Submitted, cx| this.apply_custom_baud(cx)),
            cx.subscribe(&send_input, |this, _, _: &Submitted, cx| this.send(cx)),
        ];

        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                break;
            }
        });

        let total = BUILTIN_COUNT + cfg.custom_suffixes.len();
        let mut app = Self {
            ports: vec![],
            settings: cfg.settings,
            custom_baud,
            conn: None,
            entries: vec![],
            rows: vec![],
            rx_mode: cfg.rx_mode,
            tx_mode: cfg.tx_mode,
            show_ts: cfg.show_ts,
            auto_scroll: cfg.auto_scroll,
            custom_suffixes: cfg.custom_suffixes,
            suffix_idx: if cfg.suffix_idx < total { cfg.suffix_idx } else { 3 },
            show_add_suffix: false,
            suffix_name,
            suffix_text,
            send_input,
            open_dd: None,
            status: None,
            tx_bytes: 0,
            rx_bytes: 0,
            list_scroll: UniformListScrollHandle::new(),
            _subs: subs,
            _poll: poll,
        };
        app.refresh_ports();
        app
    }

    // ---------- state helpers ----------

    fn all_suffixes(&self) -> Vec<Suffix> {
        let mut v = storage::builtin_suffixes();
        v.extend(self.custom_suffixes.iter().cloned());
        v
    }

    fn set_status(&mut self, msg: impl Into<String>, is_err: bool) {
        self.status = Some((msg.into(), is_err));
    }

    fn save(&self) {
        storage::save(&AppConfig {
            settings: self.settings.clone(),
            custom_suffixes: self.custom_suffixes.clone(),
            suffix_idx: self.suffix_idx,
            tx_mode: self.tx_mode,
            rx_mode: self.rx_mode,
            show_ts: self.show_ts,
            auto_scroll: self.auto_scroll,
        });
    }

    fn refresh_ports(&mut self) {
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

    fn mode_for(&self, dir: Dir) -> DataMode {
        match dir {
            Dir::Rx => self.rx_mode,
            Dir::Tx => self.tx_mode,
        }
    }

    fn rebuild_rows(&mut self) {
        let (rx, tx) = (self.rx_mode, self.tx_mode);
        self.rows = self
            .entries
            .iter()
            .flat_map(|e| {
                rows_for(e, if e.dir == Dir::Rx { rx } else { tx })
            })
            .collect();
        self.scroll_to_end();
    }

    fn scroll_to_end(&self) {
        if self.auto_scroll && !self.rows.is_empty() {
            self.list_scroll
                .scroll_to_item(self.rows.len() - 1, ScrollStrategy::Bottom);
        }
    }

    fn push_entry(&mut self, dir: Dir, data: Vec<u8>) {
        let entry = LogEntry { time: Local::now(), dir, data };
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

    // ---------- actions ----------

    fn drain(&mut self, cx: &mut Context<Self>) {
        let Some(conn) = &self.conn else { return };
        let mut msgs = vec![];
        while let Ok(m) = conn.rx.try_recv() {
            msgs.push(m);
        }
        if msgs.is_empty() {
            return;
        }
        for m in msgs {
            match m {
                RxMsg::Data(d) => {
                    self.rx_bytes += d.len() as u64;
                    self.push_entry(Dir::Rx, d);
                }
                RxMsg::Error(e) => {
                    self.conn = None;
                    self.set_status(format!("{e}, 串口已关闭"), true);
                }
            }
        }
        cx.notify();
    }

    fn toggle_conn(&mut self, cx: &mut Context<Self>) {
        if self.conn.take().is_some() {
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
        self.save();
        cx.notify();
    }

    fn apply_custom_baud(&mut self, cx: &mut Context<Self>) {
        if self.conn.is_some() {
            self.set_status("请先关闭串口再修改参数", true);
        } else {
            let text = self.custom_baud.read(cx).text().trim().to_string();
            match text.parse::<u32>() {
                Ok(b) if b > 0 => {
                    self.settings.baud_rate = b;
                    self.save();
                }
                _ => self.set_status("波特率必须是正整数", true),
            }
        }
        cx.notify();
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let text = self.send_input.read(cx).text().to_string();
        let mut data = match self.tx_mode {
            DataMode::Ascii => text.into_bytes(),
            DataMode::Hex => match codec::parse_hex(&text) {
                Ok(v) => v,
                Err(e) => {
                    self.set_status(e.to_string(), true);
                    cx.notify();
                    return;
                }
            },
        };
        let suffix = self.all_suffixes()[self.suffix_idx].clone();
        match codec::parse_escapes(&suffix.text) {
            Ok(s) => data.extend(s),
            Err(e) => {
                self.set_status(format!("后缀无效: {e}"), true);
                cx.notify();
                return;
            }
        }
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
            self.save();
        }
        cx.notify();
    }

    fn delete_suffix(&mut self, cx: &mut Context<Self>) {
        if self.suffix_idx >= BUILTIN_COUNT {
            self.custom_suffixes.remove(self.suffix_idx - BUILTIN_COUNT);
            self.suffix_idx = 3;
            self.save();
            cx.notify();
        }
    }

    // ---------- UI building blocks ----------

    fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Stateful<Div> {
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
    fn card(title: &str, body: impl IntoElement) -> Div {
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

    fn field(label: &str, child: impl IntoElement) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(label.to_string()))
            .child(child)
    }

    fn dropdown(
        &self,
        id: &'static str,
        which: Dd,
        current: String,
        options: Vec<String>,
        disabled: bool,
        up: bool,
        pick: PickFn,
        cx: &Context<Self>,
    ) -> Div {
        let open = self.open_dd == Some(which);
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
                this.open_dd = if this.open_dd == Some(which) { None } else { Some(which) };
                cx.notify();
            }));

        let menu = div()
            .id((id, 9999usize))
            .absolute()
            .when(up, |d| d.bottom(px(34.)))
            .when(!up, |d| d.top(px(34.)))
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
                        this.open_dd = None;
                        cx.notify();
                    }))
            }));

        div()
            .relative()
            .w_full()
            .child(trigger)
            .when(open, |d| d.child(deferred(menu).with_priority(2)))
    }

    fn seg(
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

    fn set_rx(&mut self, mode: DataMode, cx: &mut Context<Self>) {
        if self.rx_mode != mode {
            self.rx_mode = mode;
            self.rebuild_rows();
            self.save();
        }
        cx.notify();
    }

    fn set_tx(&mut self, mode: DataMode, cx: &mut Context<Self>) {
        if self.tx_mode != mode {
            self.tx_mode = mode;
            self.rebuild_rows();
            self.save();
        }
        cx.notify();
    }

    fn config_panel(&self, cx: &Context<Self>) -> Div {
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
                "dd-port",
                Dd::Port,
                port_label,
                self.ports.clone(),
                connected,
                false,
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

        let parity_idx = ParityOpt::ALL.iter().position(|p| *p == s.parity).unwrap_or(0);
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
                    .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::BOLD).child("串口调试助手")),
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
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgb(if connected { GREEN } else { 0x6b7280 })),
                    )
                    .child(if connected { "已连接" } else { "未连接" }),
            );

        let open_button = Self::button(
            "open-close",
            if connected { "关闭串口" } else { "打开串口" },
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
                    .child(open_button),
            ))
            .child(Self::card("参数", div().flex().flex_col().gap_3()
            .child(Self::field(
                "波特率",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.dropdown(
                        "dd-baud",
                        Dd::Baud,
                        s.baud_rate.to_string(),
                        BAUD_RATES.iter().map(|b| b.to_string()).collect(),
                        connected,
                        false,
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
                    "dd-data",
                    Dd::DataBits,
                    s.data_bits.to_string(),
                    (5..=8).map(|n| n.to_string()).collect(),
                    connected,
                    false,
                    |this, i, _| this.settings.data_bits = 5 + i as u8,
                    cx,
                ),
            ))
            .child(Self::field(
                "停止位",
                self.dropdown(
                    "dd-stop",
                    Dd::StopBits,
                    s.stop_bits.to_string(),
                    vec!["1".into(), "2".into()],
                    connected,
                    false,
                    |this, i, _| this.settings.stop_bits = 1 + i as u8,
                    cx,
                ),
            ))
            .child(Self::field(
                "校验位",
                self.dropdown(
                    "dd-parity",
                    Dd::Parity,
                    ParityOpt::ALL[parity_idx].label().to_string(),
                    ParityOpt::ALL.iter().map(|p| p.label().to_string()).collect(),
                    connected,
                    false,
                    |this, i, _| this.settings.parity = ParityOpt::ALL[i],
                    cx,
                ),
            ))
            .child(Self::field(
                "流控",
                self.dropdown(
                    "dd-flow",
                    Dd::Flow,
                    FlowOpt::ALL[flow_idx].label().to_string(),
                    FlowOpt::ALL.iter().map(|p| p.label().to_string()).collect(),
                    connected,
                    false,
                    |this, i, _| this.settings.flow = FlowOpt::ALL[i],
                    cx,
                ),
            ))
            ))
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
            .child(div().text_color(rgb(MUTED)).child("接收显示"))
            .child(self.seg(
                "rx-mode",
                ("ASCII", self.rx_mode == DataMode::Ascii),
                ("HEX", self.rx_mode == DataMode::Hex),
                |t, cx| t.set_rx(DataMode::Ascii, cx),
                |t, cx| t.set_rx(DataMode::Hex, cx),
                cx,
            ))
            .child(
                Self::button("ts", "时间戳", self.show_ts).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.show_ts = !this.show_ts;
                        this.save();
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::button("autoscroll", "自动滚动", self.auto_scroll).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.auto_scroll = !this.auto_scroll;
                        this.scroll_to_end();
                        this.save();
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::button("copy-all", "复制全部", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let text = export_text(&this.entries, this.rx_mode, this.tx_mode);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.set_status("日志已复制到剪贴板", false);
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::button("clear", "清空", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.entries.clear();
                        this.rows.clear();
                        this.tx_bytes = 0;
                        this.rx_bytes = 0;
                        cx.notify();
                    },
                )),
            )
    }

    fn log_view(&self, cx: &mut Context<Self>) -> Div {
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

    fn send_panel(&self, cx: &Context<Self>) -> Div {
        let suffixes = self.all_suffixes();
        let cur = suffixes[self.suffix_idx].name.clone();
        let hex_err = if self.tx_mode == DataMode::Hex {
            codec::parse_hex(self.send_input.read(cx).text()).err().map(|e| e.to_string())
        } else {
            None
        };

        let options_row = div()
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
                "dd-suffix",
                Dd::Suffix,
                cur,
                suffixes.iter().map(|s| s.name.clone()).collect(),
                false,
                true,
                |this, i, cx| {
                    this.suffix_idx = i;
                    this.save();
                    cx.notify();
                },
                cx,
            )))
            .child(
                Self::button("add-suffix", "添加后缀", self.show_add_suffix).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.show_add_suffix = !this.show_add_suffix;
                        cx.notify();
                    },
                )),
            )
            .when(self.suffix_idx >= BUILTIN_COUNT, |d| {
                d.child(
                    Self::button("del-suffix", "删除当前后缀", false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.delete_suffix(cx))),
                )
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
                        .child(
                            Self::button("suffix-ok", "确定", true)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_suffix(cx))),
                        )
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
                        d.child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .bg(rgb(if is_err { RED } else { GREEN })),
                        )
                    })
                    .child(msg),
            )
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child(format!("TX: {} B    RX: {} B", self.tx_bytes, self.rx_bytes)),
            )
    }
}

impl Render for SerialApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config_panel(cx);
        let toolbar = self.toolbar(cx);
        let log = self.log_view(cx);
        let send = self.send_panel(cx);
        let status = self.status_bar();

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(13.))
            .font_family("Microsoft YaHei UI")
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(config)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(toolbar)
                            .child(log)
                            .child(send),
                    ),
            )
            .child(status)
            .when(self.open_dd.is_some(), |d| {
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
                                    this.open_dd = None;
                                    cx.notify();
                                }),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }
}
