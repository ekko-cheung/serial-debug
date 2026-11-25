use std::ops::Range;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use gpui::{
    App, ClickEvent, ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Focusable,
    MouseButton, ScrollStrategy, SharedString, Stateful, Subscription, Task,
    UniformListScrollHandle, Window, deferred, div, prelude::*, px, rgb, uniform_list,
};

use crate::codec;
use crate::modbus::{self, ByteOrder, Decode, Fc, Request, RequestData};
use crate::model::{DataMode, Dir, LogEntry, Row, export_text, rows_for};
use crate::serial::config::{BAUD_RATES, FlowOpt, ParityOpt, PortSettings};
use crate::serial::worker::{RxMsg, SerialConn};
use crate::storage::{self, AppConfig, BUILTIN_COUNT, ModbusConfig, Suffix};
use crate::terminal::Terminal;
use crate::text_input::{Submitted, TextInput};

const MAX_ENTRIES: usize = 50_000;

/// How long to wait for a Modbus response before reporting a timeout.
const MODBUS_TIMEOUT: Duration = Duration::from_millis(1000);
const MAX_MODBUS_RESULTS: usize = 2_000;
const MODBUS_MIN_INTERVAL_MS: u64 = 100;
const MODBUS_MAX_INTERVAL_MS: u64 = 600_000;

/// Bytes sent by the Enter key in terminal mode: CR, LF, CRLF.
const EOL_BYTES: [&[u8]; 3] = [&[0x0d], &[0x0a], &[0x0d, 0x0a]];
const EOL_LABELS: [&str; 3] = ["CR", "LF", "CR+LF"];

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
    ModbusFc,
    ModbusDecode,
    ModbusOrder,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Debug,
    Terminal,
    Modbus,
}

#[derive(Clone, Copy)]
enum ExchangeState {
    Pending,
    Ok,
    Exception,
    Err,
}

struct Exchange {
    time: DateTime<Local>,
    request: String,
    state: ExchangeState,
    text: String,
}

struct Pending {
    request: Request,
    sent_at: Instant,
    exchange_idx: Option<usize>,
}

type PickFn = fn(&mut SerialApp, usize, &mut Context<SerialApp>);

pub struct SerialApp {
    ports: Vec<String>,
    settings: PortSettings,
    custom_baud: Entity<TextInput>,
    conn: Option<SerialConn>,

    mode: Mode,
    term_echo: bool,
    term_eol: usize,
    terminal: Terminal,
    term_focus: FocusHandle,

    entries: Vec<LogEntry>,
    rows: Vec<Row>,
    rx_mode: DataMode,
    tx_mode: DataMode,
    show_ts: bool,
    auto_scroll: bool,

    modbus_fc: Fc,
    modbus_slave: Entity<TextInput>,
    modbus_start: Entity<TextInput>,
    modbus_qty: Entity<TextInput>,
    modbus_value: Entity<TextInput>,
    modbus_values: Entity<TextInput>,
    modbus_interval: Entity<TextInput>,
    modbus_coil_on: bool,
    modbus_decode: Decode,
    modbus_byte_order: ByteOrder,
    modbus_poll: bool,
    modbus_results: Vec<Exchange>,
    modbus_result_scroll: UniformListScrollHandle,
    modbus_pending: Option<Pending>,
    modbus_rx_buf: Vec<u8>,
    modbus_last_sent: Option<Instant>,

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

fn new_input(cx: &mut Context<SerialApp>, placeholder: &str, initial: &str) -> Entity<TextInput> {
    cx.new(|cx| {
        let mut input = TextInput::new(cx, placeholder);
        if !initial.is_empty() {
            input.set_text(initial, cx);
        }
        input
    })
}

fn parse_u16_field(text: &str, label: &str) -> Result<u16, String> {
    let value = modbus::parse_number(text).map_err(|e| format!("{label}: {e}"))?;
    if value > u16::MAX as u32 {
        return Err(format!("{label}超出 0..=65535"));
    }
    Ok(value as u16)
}

impl SerialApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cfg = storage::load();
        let mb = cfg.modbus.clone();

        let custom_baud = cx.new(|cx| TextInput::new(cx, "自定义波特率, 回车应用"));
        let suffix_name = cx.new(|cx| TextInput::new(cx, "后缀名称"));
        let suffix_text = cx.new(|cx| TextInput::new(cx, "后缀内容, 如 \\r\\n 或 \\x0D\\x0A"));
        let send_input = cx.new(|cx| TextInput::new(cx, "输入要发送的内容, 回车发送"));

        let modbus_slave = new_input(cx, "1", &mb.slave);
        let modbus_start = new_input(cx, "0", &mb.start);
        let modbus_qty = new_input(cx, "1", &mb.qty);
        let modbus_value = new_input(cx, "0", &mb.value);
        let modbus_values = new_input(cx, "0 1 2", &mb.values);
        let modbus_interval = new_input(cx, "1000", &mb.poll_interval);

        let subs = vec![
            cx.subscribe(&custom_baud, |this, _, _: &Submitted, cx| {
                this.apply_custom_baud(cx)
            }),
            cx.subscribe(&send_input, |this, _, _: &Submitted, cx| this.send(cx)),
            cx.subscribe(&modbus_slave, |this, _, _: &Submitted, cx| {
                this.send_modbus(cx)
            }),
            cx.subscribe(&modbus_start, |this, _, _: &Submitted, cx| {
                this.send_modbus(cx)
            }),
            cx.subscribe(&modbus_qty, |this, _, _: &Submitted, cx| {
                this.send_modbus(cx)
            }),
            cx.subscribe(&modbus_value, |this, _, _: &Submitted, cx| {
                this.send_modbus(cx)
            }),
            cx.subscribe(&modbus_values, |this, _, _: &Submitted, cx| {
                this.send_modbus(cx)
            }),
        ];

        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                    break;
                }
            }
        });

        let total = BUILTIN_COUNT + cfg.custom_suffixes.len();
        let mut app = Self {
            ports: vec![],
            settings: cfg.settings,
            custom_baud,
            conn: None,
            mode: Mode::Debug,
            term_echo: false,
            term_eol: 0,
            terminal: Terminal::new(),
            term_focus: cx.focus_handle(),
            entries: vec![],
            rows: vec![],
            rx_mode: cfg.rx_mode,
            tx_mode: cfg.tx_mode,
            show_ts: cfg.show_ts,
            auto_scroll: cfg.auto_scroll,
            modbus_fc: Fc::from_code(mb.fc).unwrap_or(Fc::ReadHoldingRegisters),
            modbus_slave,
            modbus_start,
            modbus_qty,
            modbus_value,
            modbus_values,
            modbus_interval,
            modbus_coil_on: mb.coil_on,
            modbus_decode: mb.decode,
            modbus_byte_order: mb.byte_order,
            modbus_poll: false,
            modbus_results: vec![],
            modbus_result_scroll: UniformListScrollHandle::new(),
            modbus_pending: None,
            modbus_rx_buf: vec![],
            modbus_last_sent: None,
            custom_suffixes: cfg.custom_suffixes,
            suffix_idx: if cfg.suffix_idx < total {
                cfg.suffix_idx
            } else {
                3
            },
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

    fn save(&self, cx: &App) {
        storage::save(&AppConfig {
            settings: self.settings.clone(),
            custom_suffixes: self.custom_suffixes.clone(),
            suffix_idx: self.suffix_idx,
            tx_mode: self.tx_mode,
            rx_mode: self.rx_mode,
            show_ts: self.show_ts,
            auto_scroll: self.auto_scroll,
            modbus: ModbusConfig {
                slave: self.modbus_slave.read(cx).text().to_string(),
                fc: self.modbus_fc.code(),
                start: self.modbus_start.read(cx).text().to_string(),
                qty: self.modbus_qty.read(cx).text().to_string(),
                value: self.modbus_value.read(cx).text().to_string(),
                values: self.modbus_values.read(cx).text().to_string(),
                poll_interval: self.modbus_interval.read(cx).text().to_string(),
                coil_on: self.modbus_coil_on,
                decode: self.modbus_decode,
                byte_order: self.modbus_byte_order,
            },
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
        if self.mode == Mode::Modbus {
            // Modbus frames are always shown as HEX, regardless of the debug modes.
            return DataMode::Hex;
        }
        match dir {
            Dir::Rx => self.rx_mode,
            Dir::Tx => self.tx_mode,
        }
    }

    fn rebuild_rows(&mut self) {
        let rows: Vec<Row> = self
            .entries
            .iter()
            .flat_map(|e| rows_for(e, self.mode_for(e.dir)))
            .collect();
        self.rows = rows;
        self.scroll_to_end();
    }

    fn scroll_to_end(&self) {
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
                if !self.modbus_results.is_empty() {
                    self.modbus_result_scroll
                        .scroll_to_item(self.modbus_results.len() - 1, ScrollStrategy::Bottom);
                }
            }
        }
    }

    fn push_entry(&mut self, dir: Dir, data: Vec<u8>) {
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

    // ---------- actions ----------

    fn drain(&mut self, cx: &mut Context<Self>) {
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
                    self.abort_modbus("串口已关闭");
                    self.modbus_poll = false;
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

    // ---------- Modbus master ----------

    fn build_modbus_request(&self, cx: &App) -> Result<Request, String> {
        let slave = modbus::parse_number(self.modbus_slave.read(cx).text())
            .map_err(|e| format!("从站地址: {e}"))?;
        if slave > 247 {
            return Err("从站地址必须在 0..=247（0 为广播）".into());
        }
        let start = parse_u16_field(self.modbus_start.read(cx).text(), "起始地址")?;
        let fc = self.modbus_fc;
        let req = match fc {
            Fc::ReadCoils
            | Fc::ReadDiscreteInputs
            | Fc::ReadHoldingRegisters
            | Fc::ReadInputRegisters => {
                let quantity = parse_u16_field(self.modbus_qty.read(cx).text(), "数量")?;
                Request {
                    slave: slave as u8,
                    fc,
                    address: start,
                    quantity,
                    data: RequestData::Read,
                }
            }
            Fc::WriteSingleCoil => Request {
                slave: slave as u8,
                fc,
                address: start,
                quantity: 1,
                data: RequestData::SingleCoil(self.modbus_coil_on),
            },
            Fc::WriteSingleRegister => Request {
                slave: slave as u8,
                fc,
                address: start,
                quantity: 1,
                data: RequestData::SingleRegister(parse_u16_field(
                    self.modbus_value.read(cx).text(),
                    "寄存器值",
                )?),
            },
            Fc::WriteMultipleCoils => {
                let values = modbus::parse_coil_list(self.modbus_values.read(cx).text())
                    .map_err(|e| format!("线圈值: {e}"))?;
                if values.len() > u16::MAX as usize {
                    return Err("线圈数量过多".into());
                }
                Request {
                    slave: slave as u8,
                    fc,
                    address: start,
                    quantity: values.len() as u16,
                    data: RequestData::Coils(values),
                }
            }
            Fc::WriteMultipleRegisters => {
                let values = modbus::parse_u16_list(self.modbus_values.read(cx).text())
                    .map_err(|e| format!("寄存器值: {e}"))?;
                if values.len() > u16::MAX as usize {
                    return Err("寄存器数量过多".into());
                }
                Request {
                    slave: slave as u8,
                    fc,
                    address: start,
                    quantity: values.len() as u16,
                    data: RequestData::Registers(values),
                }
            }
        };
        req.validate()?;
        Ok(req)
    }

    fn modbus_interval(&self, cx: &App) -> Result<Duration, String> {
        let ms = modbus::parse_number(self.modbus_interval.read(cx).text())
            .map_err(|e| format!("轮询间隔: {e}"))? as u64;
        if !(MODBUS_MIN_INTERVAL_MS..=MODBUS_MAX_INTERVAL_MS).contains(&ms) {
            return Err(format!(
                "轮询间隔必须在 {MODBUS_MIN_INTERVAL_MS}..={MODBUS_MAX_INTERVAL_MS} ms"
            ));
        }
        Ok(Duration::from_millis(ms))
    }

    fn send_modbus(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Modbus {
            return;
        }
        self.send_modbus_now(cx);
        self.save(cx);
        cx.notify();
    }

    /// Build and write one request. Returns true when the UI should redraw.
    fn send_modbus_now(&mut self, cx: &mut Context<Self>) -> bool {
        if self.modbus_pending.is_some() {
            self.set_status("正在等待从站响应", true);
            return true;
        }
        if self.conn.is_none() {
            self.modbus_poll = false;
            self.set_status("串口未打开", true);
            return true;
        }
        let req = match self.build_modbus_request(cx) {
            Ok(req) => req,
            Err(e) => {
                self.modbus_poll = false;
                self.set_status(e, true);
                return true;
            }
        };
        let adu = match req.adu() {
            Ok(adu) => adu,
            Err(e) => {
                self.modbus_poll = false;
                self.set_status(e, true);
                return true;
            }
        };
        let written = self.conn.as_mut().expect("checked above").write(&adu);
        if let Err(e) = written {
            self.modbus_poll = false;
            self.set_status(e.to_string(), true);
            return true;
        }

        self.tx_bytes += adu.len() as u64;
        self.push_entry(Dir::Tx, adu);
        self.modbus_rx_buf.clear();
        let now = Instant::now();
        self.modbus_last_sent = Some(now);
        self.modbus_results.push(Exchange {
            time: Local::now(),
            request: modbus::describe_request(&req),
            state: ExchangeState::Pending,
            text: "等待响应…".into(),
        });
        self.trim_modbus_results();
        let idx = self.modbus_results.len() - 1;
        if req.slave == 0 {
            self.modbus_results[idx].state = ExchangeState::Ok;
            self.modbus_results[idx].text = "广播已发送（从站不应答）".into();
            self.modbus_pending = None;
            self.set_status("广播已发送", false);
        } else {
            self.modbus_pending = Some(Pending {
                request: req,
                sent_at: now,
                exchange_idx: Some(idx),
            });
            self.set_status("已发送，等待响应…", false);
        }
        self.scroll_to_end();
        true
    }

    fn handle_modbus_rx(&mut self, data: &[u8]) -> bool {
        let Some(req) = self.modbus_pending.as_ref().map(|p| p.request.clone()) else {
            self.push_entry(Dir::Rx, data.to_vec());
            return true;
        };
        self.modbus_rx_buf.extend_from_slice(data);
        match modbus::try_parse_response(&self.modbus_rx_buf, &req) {
            modbus::Progress::NeedMore => false,
            modbus::Progress::Ready(resp) => {
                let raw = resp.raw.clone();
                self.modbus_rx_buf.drain(..raw.len());
                self.push_entry(Dir::Rx, raw);
                let text =
                    modbus::describe_response(&resp, self.modbus_decode, self.modbus_byte_order);
                self.finish_modbus(ExchangeState::Ok, text);
                true
            }
            modbus::Progress::Exception { code, raw } => {
                self.modbus_rx_buf.drain(..raw.len());
                self.push_entry(Dir::Rx, raw);
                let text = format!("从站异常 0x{code:02X}: {}", modbus::exception_name(code));
                self.finish_modbus(ExchangeState::Exception, text);
                true
            }
            modbus::Progress::Invalid(err) => {
                let raw = std::mem::take(&mut self.modbus_rx_buf);
                if !raw.is_empty() {
                    self.push_entry(Dir::Rx, raw);
                }
                self.finish_modbus(ExchangeState::Err, err);
                true
            }
        }
    }

    fn finish_modbus(&mut self, state: ExchangeState, text: String) {
        let Some(pending) = self.modbus_pending.take() else {
            return;
        };
        let updated = pending
            .exchange_idx
            .and_then(|idx| self.modbus_results.get_mut(idx))
            .filter(|ex| matches!(ex.state, ExchangeState::Pending))
            .map(|ex| {
                ex.state = state;
                ex.text = text.clone();
            });
        if updated.is_none() {
            self.modbus_results.push(Exchange {
                time: Local::now(),
                request: modbus::describe_request(&pending.request),
                state,
                text,
            });
            self.trim_modbus_results();
        }
        self.scroll_to_end();
    }

    fn abort_modbus(&mut self, reason: &str) {
        if self.modbus_pending.is_none() {
            return;
        }
        let raw = std::mem::take(&mut self.modbus_rx_buf);
        if !raw.is_empty() {
            self.push_entry(Dir::Rx, raw);
        }
        self.finish_modbus(ExchangeState::Err, reason.to_string());
    }

    fn trim_modbus_results(&mut self) {
        if self.modbus_results.len() <= MAX_MODBUS_RESULTS {
            return;
        }
        let drop = MAX_MODBUS_RESULTS / 10;
        self.modbus_results.drain(..drop);
        if let Some(pending) = &mut self.modbus_pending
            && let Some(idx) = pending.exchange_idx
        {
            pending.exchange_idx = idx.checked_sub(drop);
        }
    }

    fn modbus_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(sent_at) = self.modbus_pending.as_ref().map(|p| p.sent_at)
            && sent_at.elapsed() >= MODBUS_TIMEOUT
        {
            let raw = std::mem::take(&mut self.modbus_rx_buf);
            if !raw.is_empty() {
                self.push_entry(Dir::Rx, raw);
            }
            self.finish_modbus(
                ExchangeState::Err,
                format!("响应超时 (>{} ms)", MODBUS_TIMEOUT.as_millis()),
            );
            self.set_status("Modbus 响应超时", true);
            changed = true;
        }
        if self.modbus_poll && self.modbus_pending.is_none() && self.conn.is_some() {
            match self.modbus_interval(cx) {
                Ok(interval) => {
                    if self
                        .modbus_last_sent
                        .is_none_or(|t| t.elapsed() >= interval)
                    {
                        changed |= self.send_modbus_now(cx);
                    }
                }
                Err(e) => {
                    self.modbus_poll = false;
                    self.set_status(e, true);
                    changed = true;
                }
            }
        }
        changed
    }

    fn toggle_modbus_poll(&mut self, cx: &mut Context<Self>) {
        if self.modbus_poll {
            self.modbus_poll = false;
            self.set_status("轮询已停止", false);
        } else if self.conn.is_none() {
            self.set_status("请先打开串口", true);
        } else if let Err(e) = self.modbus_interval(cx) {
            self.set_status(e, true);
        } else if let Err(e) = self.build_modbus_request(cx) {
            self.set_status(e, true);
        } else {
            self.modbus_poll = true;
            self.modbus_last_sent = None;
            self.send_modbus_now(cx);
        }
        self.save(cx);
        cx.notify();
    }

    /// Terminal mode: translate a key press into the bytes a terminal would send.
    fn terminal_key(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        let m = &ks.modifiers;
        let mut data: Vec<u8> = match ks.key.as_str() {
            "enter" => EOL_BYTES[self.term_eol].to_vec(),
            "backspace" => vec![0x7f],
            "tab" => vec![0x09],
            "escape" => vec![0x1b],
            "up" => b"\x1b[A".to_vec(),
            "down" => b"\x1b[B".to_vec(),
            "right" => b"\x1b[C".to_vec(),
            "left" => b"\x1b[D".to_vec(),
            "home" => b"\x1b[H".to_vec(),
            "end" => b"\x1b[F".to_vec(),
            "delete" => b"\x1b[3~".to_vec(),
            "pageup" => b"\x1b[5~".to_vec(),
            "pagedown" => b"\x1b[6~".to_vec(),
            "v" if m.control && m.shift => match cx.read_from_clipboard().and_then(|c| c.text()) {
                Some(t) => t.into_bytes(),
                None => return,
            },
            k if m.control && k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic() => {
                vec![k.as_bytes()[0].to_ascii_lowercase() - b'a' + 1]
            }
            _ if m.control || m.platform => return,
            _ => match &ks.key_char {
                Some(c) => c.clone().into_bytes(),
                None => return,
            },
        };
        if m.alt && ks.key_char.is_some() {
            data.insert(0, 0x1b);
        }
        let Some(conn) = self.conn.as_mut() else {
            self.set_status("串口未打开", true);
            cx.notify();
            return;
        };
        match conn.write(&data) {
            Ok(()) => {
                self.tx_bytes += data.len() as u64;
                if self.term_echo {
                    let echo: &[u8] = match ks.key.as_str() {
                        "enter" => b"\r\n",
                        "backspace" => b"\x08\x1b[P",
                        _ if ks.key_char.is_some() || (m.control && m.shift) => &data,
                        _ => &[],
                    };
                    self.terminal.feed(echo);
                    self.scroll_to_end();
                }
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        if self.mode == Mode::Modbus {
            self.abort_modbus("已切换到其他模式");
            self.modbus_poll = false;
        }
        self.mode = mode;
        self.open_dd = None;
        let focus = match mode {
            Mode::Terminal => self.term_focus.clone(),
            Mode::Debug => self.send_input.read(cx).focus_handle(cx),
            Mode::Modbus => self.modbus_start.read(cx).focus_handle(cx),
        };
        window.focus(&focus);
        self.rebuild_rows();
        self.scroll_to_end();
        self.save(cx);
        cx.notify();
    }

    fn toggle_conn(&mut self, cx: &mut Context<Self>) {
        if self.conn.take().is_some() {
            self.abort_modbus("串口已关闭");
            self.modbus_poll = false;
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

    fn apply_custom_baud(&mut self, cx: &mut Context<Self>) {
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

    // ---------- UI building blocks ----------

    fn button(
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
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(label.to_string()),
            )
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
                this.open_dd = if this.open_dd == Some(which) {
                    None
                } else {
                    Some(which)
                };
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
            self.save(cx);
        }
        cx.notify();
    }

    fn set_tx(&mut self, mode: DataMode, cx: &mut Context<Self>) {
        if self.tx_mode != mode {
            self.tx_mode = mode;
            self.rebuild_rows();
            self.save(cx);
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
                            ParityOpt::ALL
                                .iter()
                                .map(|p| p.label().to_string())
                                .collect(),
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
                    )),
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
                    this.abort_modbus("已清空");
                    this.entries.clear();
                    this.rows.clear();
                    this.terminal.clear();
                    this.modbus_results.clear();
                    this.modbus_rx_buf.clear();
                    this.tx_bytes = 0;
                    this.rx_bytes = 0;
                    cx.notify();
                },
            )))
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

    fn terminal_view(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .m_3()
            .relative()
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x0c0e11))
            .font_family("Consolas")
            .text_size(px(13.))
            .child(
                div()
                    .id("terminal")
                    .size_full()
                    .track_focus(&self.term_focus)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, w, _| w.focus(&this.term_focus)),
                    )
                    .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                        this.terminal_key(&e.keystroke, cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        uniform_list(
                            "term-lines",
                            self.terminal.line_count(),
                            cx.processor(|this, range: Range<usize>, w: &mut Window, _cx| {
                                let cursor = this.term_focus.is_focused(w);
                                range
                                    .map(|ix| {
                                        div()
                                            .h(px(20.))
                                            .px_3()
                                            .whitespace_nowrap()
                                            .flex()
                                            .text_color(rgb(TEXT))
                                            .children(
                                                this.terminal.runs(ix, cursor).into_iter().map(
                                                    |(text, fg)| {
                                                        div()
                                                            .flex_none()
                                                            .text_color(rgb(fg.unwrap_or(TEXT)))
                                                            .child(text)
                                                    },
                                                ),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(self.list_scroll.clone())
                        .size_full(),
                    ),
            )
    }

    fn modbus_export(&self) -> String {
        let mut out = String::new();
        for ex in &self.modbus_results {
            out.push_str(&format!(
                "[{}] {} -> {}\n",
                ex.time.format("%H:%M:%S%.3f"),
                ex.request,
                ex.text
            ));
        }
        out.push_str("\n--- 原始帧 (HEX) ---\n");
        out.push_str(&export_text(&self.entries, DataMode::Hex, DataMode::Hex));
        out
    }

    fn modbus_results_view(&self, cx: &mut Context<Self>) -> Div {
        let count = self.modbus_results.len();
        div()
            .flex_1()
            .min_h_0()
            .m_3()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x101216))
            .child(
                div()
                    .flex_none()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(rgb(PANEL))
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(format!("解析结果 ({count}) · 下方日志为原始 HEX 帧")),
                    )
                    .child(
                        Self::button("mb-clear-res", "清空结果", false)
                            .h(px(22.))
                            .text_size(px(11.))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.abort_modbus("结果已清空");
                                this.modbus_results.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when(count == 0, |d| {
                        d.child(
                            div()
                                .absolute()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(0x4b5260))
                                .text_size(px(12.))
                                .child("发送 Modbus 请求后在此显示解析结果"),
                        )
                    })
                    .child(
                        uniform_list(
                            "mb-results",
                            count,
                            cx.processor(|this, range: Range<usize>, _w: &mut Window, _cx| {
                                range
                                    .map(|ix| {
                                        let ex = &this.modbus_results[ix];
                                        let color = match ex.state {
                                            ExchangeState::Pending => MUTED,
                                            ExchangeState::Ok => GREEN,
                                            ExchangeState::Exception => ORANGE,
                                            ExchangeState::Err => RED,
                                        };
                                        div()
                                            .h(px(38.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .when(ix % 2 == 1, |d| d.bg(rgb(0x14171c)))
                                            .child(
                                                div()
                                                    .w(px(78.))
                                                    .flex_none()
                                                    .text_size(px(11.))
                                                    .text_color(rgb(0x6b7280))
                                                    .child(
                                                        ex.time.format("%H:%M:%S%.3f").to_string(),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(rgb(MUTED))
                                                            .truncate()
                                                            .child(ex.request.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .text_color(rgb(color))
                                                            .truncate()
                                                            .child(ex.text.clone()),
                                                    ),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(self.modbus_result_scroll.clone())
                        .size_full(),
                    ),
            )
    }

    fn modbus_panel(&self, cx: &Context<Self>) -> Div {
        let fc = self.modbus_fc;
        let err = self.build_modbus_request(cx).err();
        let connected = self.conn.is_some();

        let fc_idx = Fc::ALL.iter().position(|f| *f == fc).unwrap_or(2);

        let mut row = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_2()
            .child(Self::field(
                "从站",
                div().w(px(64.)).child(self.modbus_slave.clone()),
            ))
            .child(Self::field(
                "功能码",
                div().w(px(170.)).child(self.dropdown(
                    "dd-mb-fc",
                    Dd::ModbusFc,
                    Fc::ALL[fc_idx].label().to_string(),
                    Fc::ALL.iter().map(|f| f.label().to_string()).collect(),
                    false,
                    true,
                    |this, i, _| this.modbus_fc = Fc::ALL[i],
                    cx,
                )),
            ))
            .child(Self::field(
                "起始地址",
                div().w(px(96.)).child(self.modbus_start.clone()),
            ));

        if fc.is_read() {
            row = row.child(Self::field(
                "数量",
                div().w(px(76.)).child(self.modbus_qty.clone()),
            ));
        } else if fc == Fc::WriteSingleCoil {
            row = row.child(Self::field(
                "值",
                self.seg(
                    "mb-coil",
                    ("ON", self.modbus_coil_on),
                    ("OFF", !self.modbus_coil_on),
                    |this, cx| {
                        this.modbus_coil_on = true;
                        this.save(cx);
                        cx.notify();
                    },
                    |this, cx| {
                        this.modbus_coil_on = false;
                        this.save(cx);
                        cx.notify();
                    },
                    cx,
                ),
            ));
        } else if fc == Fc::WriteSingleRegister {
            row = row.child(Self::field(
                "值",
                div().w(px(96.)).child(self.modbus_value.clone()),
            ));
        }

        row = row
            .child(Self::field(
                "操作",
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Self::button("mb-send", "发送", true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.send_modbus(cx)),
                        ),
                    )
                    .child(
                        Self::button(
                            "mb-poll",
                            if self.modbus_poll {
                                "停止轮询"
                            } else {
                                "周期轮询"
                            },
                            self.modbus_poll,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_modbus_poll(cx)),
                        ),
                    ),
            ))
            .child(Self::field(
                "间隔 ms",
                div().w(px(76.)).child(self.modbus_interval.clone()),
            ));

        let mut extra = div().flex().flex_wrap().items_end().gap_3();
        let has_extra = match fc {
            Fc::WriteMultipleCoils => {
                extra = extra.child(div().flex_1().min_w_0().child(Self::field(
                    "线圈值 (0/1 序列，如 1 0 1 1)",
                    self.modbus_values.clone(),
                )));
                true
            }
            Fc::WriteMultipleRegisters => {
                extra = extra.child(div().flex_1().min_w_0().child(Self::field(
                    "寄存器值 (十进制或 0x，如 0 100 0x1F)",
                    self.modbus_values.clone(),
                )));
                true
            }
            Fc::ReadHoldingRegisters | Fc::ReadInputRegisters => {
                extra = extra
                    .child(Self::field(
                        "解码",
                        div().w(px(104.)).child(self.dropdown(
                            "dd-mb-decode",
                            Dd::ModbusDecode,
                            self.modbus_decode.label().to_string(),
                            Decode::ALL.iter().map(|d| d.label().to_string()).collect(),
                            false,
                            true,
                            |this, i, cx| {
                                this.modbus_decode = Decode::ALL[i];
                                this.save(cx);
                            },
                            cx,
                        )),
                    ))
                    .when(self.modbus_decode.is_32bit(), |d| {
                        d.child(Self::field(
                            "字节序",
                            div().w(px(150.)).child(
                                self.dropdown(
                                    "dd-mb-order",
                                    Dd::ModbusOrder,
                                    self.modbus_byte_order.label().to_string(),
                                    ByteOrder::ALL
                                        .iter()
                                        .map(|o| o.label().to_string())
                                        .collect(),
                                    false,
                                    true,
                                    |this, i, cx| {
                                        this.modbus_byte_order = ByteOrder::ALL[i];
                                        this.save(cx);
                                    },
                                    cx,
                                ),
                            ),
                        ))
                    });
                true
            }
            _ => false,
        };

        let hint = match fc {
            Fc::ReadCoils | Fc::ReadDiscreteInputs => {
                "数量 1..=2000；Modbus RTU 通常使用 8N1 或 8E1（在左侧设置）"
            }
            Fc::ReadHoldingRegisters | Fc::ReadInputRegisters => {
                "数量 1..=125；32 位解码按每两个寄存器组合"
            }
            Fc::WriteSingleCoil => "ON = 0xFF00，OFF = 0x0000；从站地址 0 表示广播",
            Fc::WriteSingleRegister => "值范围 0..=65535；从站地址 0 表示广播",
            Fc::WriteMultipleCoils => "数量由 0/1 序列长度决定，最多 1968 个；CRC 自动计算",
            Fc::WriteMultipleRegisters => "数量由值列表长度决定，最多 123 个；CRC 自动计算",
        };

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(row)
            .when(has_extra, |d| d.child(extra))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(hint))
                    .when(!connected, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(ORANGE))
                                .child("串口未打开"),
                        )
                    })
                    .when_some(err, |d, e| {
                        d.child(div().text_size(px(11.)).text_color(rgb(RED)).child(e))
                    }),
            )
    }

    fn send_panel(&self, cx: &Context<Self>) -> Div {
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
                    "dd-suffix",
                    Dd::Suffix,
                    cur,
                    suffixes.iter().map(|s| s.name.clone()).collect(),
                    false,
                    true,
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

        let main = match self.mode {
            Mode::Terminal => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(toolbar)
                .child(self.terminal_view(cx)),
            Mode::Debug => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(toolbar)
                .child(self.log_view(cx))
                .child(self.send_panel(cx)),
            Mode::Modbus => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(toolbar)
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
