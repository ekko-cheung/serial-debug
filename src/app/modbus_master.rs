mod view;

use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, ScrollStrategy, Subscription,
    UniformListScrollHandle,
};

use crate::modbus::{self, ByteOrder, Decode, Fc, Request, RequestData};
use crate::model::{DataMode, Dir, export_text};
use crate::storage::ModbusConfig;
use crate::text_input::{Submitted, TextInput};

use super::{Mode, SerialApp};

/// How long to wait for a Modbus response before reporting a timeout.
const MODBUS_TIMEOUT: Duration = Duration::from_millis(1000);
const MAX_MODBUS_RESULTS: usize = 2_000;
const MODBUS_MIN_INTERVAL_MS: u64 = 100;
const MODBUS_MAX_INTERVAL_MS: u64 = 600_000;

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

pub(super) struct ModbusMaster {
    fc: Fc,
    slave: Entity<TextInput>,
    start: Entity<TextInput>,
    qty: Entity<TextInput>,
    value: Entity<TextInput>,
    values: Entity<TextInput>,
    poll_interval: Entity<TextInput>,
    coil_on: bool,
    decode: Decode,
    byte_order: ByteOrder,
    polling: bool,
    results: Vec<Exchange>,
    result_scroll: UniformListScrollHandle,
    pending: Option<Pending>,
    rx_buf: Vec<u8>,
    last_sent: Option<Instant>,
}

impl ModbusMaster {
    pub(super) fn new(cx: &mut Context<SerialApp>, cfg: &ModbusConfig) -> Self {
        let mut input = |placeholder: &str, initial: &str| {
            cx.new(|cx| {
                let mut input = TextInput::new(cx, placeholder);
                if !initial.is_empty() {
                    input.set_text(initial, cx);
                }
                input
            })
        };

        Self {
            fc: Fc::from_code(cfg.fc).unwrap_or(Fc::ReadHoldingRegisters),
            slave: input("1", &cfg.slave),
            start: input("0", &cfg.start),
            qty: input("1", &cfg.qty),
            value: input("0", &cfg.value),
            values: input("0 1 2", &cfg.values),
            poll_interval: input("1000", &cfg.poll_interval),
            coil_on: cfg.coil_on,
            decode: cfg.decode,
            byte_order: cfg.byte_order,
            polling: false,
            results: Vec::new(),
            result_scroll: UniformListScrollHandle::new(),
            pending: None,
            rx_buf: Vec::new(),
            last_sent: None,
        }
    }

    pub(super) fn subscribe(&self, cx: &mut Context<SerialApp>) -> [Subscription; 5] {
        [
            &self.slave,
            &self.start,
            &self.qty,
            &self.value,
            &self.values,
        ]
        .map(|input| cx.subscribe(input, |this, _, _: &Submitted, cx| this.send_modbus(cx)))
    }

    pub(super) fn config(&self, cx: &App) -> ModbusConfig {
        ModbusConfig {
            slave: self.slave.read(cx).text().to_string(),
            fc: self.fc.code(),
            start: self.start.read(cx).text().to_string(),
            qty: self.qty.read(cx).text().to_string(),
            value: self.value.read(cx).text().to_string(),
            values: self.values.read(cx).text().to_string(),
            poll_interval: self.poll_interval.read(cx).text().to_string(),
            coil_on: self.coil_on,
            decode: self.decode,
            byte_order: self.byte_order,
        }
    }

    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.start.read(cx).focus_handle(cx)
    }

    pub(super) fn scroll_to_end(&self) {
        if !self.results.is_empty() {
            self.result_scroll
                .scroll_to_item(self.results.len() - 1, ScrollStrategy::Bottom);
        }
    }

    fn build_request(&self, cx: &App) -> Result<Request, String> {
        let slave = modbus::parse_number(self.slave.read(cx).text())
            .map_err(|e| format!("从站地址: {e}"))?;
        if slave > 247 {
            return Err("从站地址必须在 0..=247（0 为广播）".into());
        }
        let start = parse_u16_field(self.start.read(cx).text(), "起始地址")?;
        let fc = self.fc;
        let req = match fc {
            Fc::ReadCoils
            | Fc::ReadDiscreteInputs
            | Fc::ReadHoldingRegisters
            | Fc::ReadInputRegisters => {
                let quantity = parse_u16_field(self.qty.read(cx).text(), "数量")?;
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
                data: RequestData::SingleCoil(self.coil_on),
            },
            Fc::WriteSingleRegister => Request {
                slave: slave as u8,
                fc,
                address: start,
                quantity: 1,
                data: RequestData::SingleRegister(parse_u16_field(
                    self.value.read(cx).text(),
                    "寄存器值",
                )?),
            },
            Fc::WriteMultipleCoils => {
                let values = modbus::parse_coil_list(self.values.read(cx).text())
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
                let values = modbus::parse_u16_list(self.values.read(cx).text())
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

    fn interval(&self, cx: &App) -> Result<Duration, String> {
        let ms = modbus::parse_number(self.poll_interval.read(cx).text())
            .map_err(|e| format!("轮询间隔: {e}"))? as u64;
        if !(MODBUS_MIN_INTERVAL_MS..=MODBUS_MAX_INTERVAL_MS).contains(&ms) {
            return Err(format!(
                "轮询间隔必须在 {MODBUS_MIN_INTERVAL_MS}..={MODBUS_MAX_INTERVAL_MS} ms"
            ));
        }
        Ok(Duration::from_millis(ms))
    }

    fn trim_results(&mut self) {
        if self.results.len() <= MAX_MODBUS_RESULTS {
            return;
        }
        let drop = MAX_MODBUS_RESULTS / 10;
        self.results.drain(..drop);
        if let Some(pending) = &mut self.pending
            && let Some(idx) = pending.exchange_idx
        {
            pending.exchange_idx = idx.checked_sub(drop);
        }
    }
}

fn parse_u16_field(text: &str, label: &str) -> Result<u16, String> {
    let value = modbus::parse_number(text).map_err(|e| format!("{label}: {e}"))?;
    if value > u16::MAX as u32 {
        return Err(format!("{label}超出 0..=65535"));
    }
    Ok(value as u16)
}

impl SerialApp {
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
        if self.modbus.pending.is_some() {
            self.set_status("正在等待从站响应", true);
            return true;
        }
        if self.conn.is_none() {
            self.modbus.polling = false;
            self.set_status("串口未打开", true);
            return true;
        }
        let req = match self.modbus.build_request(cx) {
            Ok(req) => req,
            Err(e) => {
                self.modbus.polling = false;
                self.set_status(e, true);
                return true;
            }
        };
        let adu = match req.adu() {
            Ok(adu) => adu,
            Err(e) => {
                self.modbus.polling = false;
                self.set_status(e, true);
                return true;
            }
        };
        let written = self.conn.as_mut().expect("checked above").write(&adu);
        if let Err(e) = written {
            self.modbus.polling = false;
            self.set_status(e.to_string(), true);
            return true;
        }

        self.tx_bytes += adu.len() as u64;
        self.push_entry(Dir::Tx, adu);
        self.modbus.rx_buf.clear();
        let now = Instant::now();
        self.modbus.last_sent = Some(now);
        self.modbus.results.push(Exchange {
            time: Local::now(),
            request: modbus::describe_request(&req),
            state: ExchangeState::Pending,
            text: "等待响应…".into(),
        });
        self.modbus.trim_results();
        let idx = self.modbus.results.len() - 1;
        if req.slave == 0 {
            self.modbus.results[idx].state = ExchangeState::Ok;
            self.modbus.results[idx].text = "广播已发送（从站不应答）".into();
            self.modbus.pending = None;
            self.set_status("广播已发送", false);
        } else {
            self.modbus.pending = Some(Pending {
                request: req,
                sent_at: now,
                exchange_idx: Some(idx),
            });
            self.set_status("已发送，等待响应…", false);
        }
        self.scroll_to_end();
        true
    }

    pub(super) fn handle_modbus_rx(&mut self, data: &[u8]) -> bool {
        let Some(req) = self.modbus.pending.as_ref().map(|p| p.request.clone()) else {
            self.push_entry(Dir::Rx, data.to_vec());
            return true;
        };
        self.modbus.rx_buf.extend_from_slice(data);
        match modbus::try_parse_response(&self.modbus.rx_buf, &req) {
            modbus::Progress::NeedMore => false,
            modbus::Progress::Ready(resp) => {
                let raw = resp.raw.clone();
                self.modbus.rx_buf.drain(..raw.len());
                self.push_entry(Dir::Rx, raw);
                let text =
                    modbus::describe_response(&resp, self.modbus.decode, self.modbus.byte_order);
                self.finish_modbus(ExchangeState::Ok, text);
                true
            }
            modbus::Progress::Exception { code, raw } => {
                self.modbus.rx_buf.drain(..raw.len());
                self.push_entry(Dir::Rx, raw);
                let text = format!("从站异常 0x{code:02X}: {}", modbus::exception_name(code));
                self.finish_modbus(ExchangeState::Exception, text);
                true
            }
            modbus::Progress::Invalid(err) => {
                let raw = std::mem::take(&mut self.modbus.rx_buf);
                if !raw.is_empty() {
                    self.push_entry(Dir::Rx, raw);
                }
                self.finish_modbus(ExchangeState::Err, err);
                true
            }
        }
    }

    fn finish_modbus(&mut self, state: ExchangeState, text: String) {
        let Some(pending) = self.modbus.pending.take() else {
            return;
        };
        let updated = pending
            .exchange_idx
            .and_then(|idx| self.modbus.results.get_mut(idx))
            .filter(|ex| matches!(ex.state, ExchangeState::Pending))
            .map(|ex| {
                ex.state = state;
                ex.text = text.clone();
            });
        if updated.is_none() {
            self.modbus.results.push(Exchange {
                time: Local::now(),
                request: modbus::describe_request(&pending.request),
                state,
                text,
            });
            self.modbus.trim_results();
        }
        self.scroll_to_end();
    }

    fn abort_modbus(&mut self, reason: &str) {
        if self.modbus.pending.is_none() {
            return;
        }
        let raw = std::mem::take(&mut self.modbus.rx_buf);
        if !raw.is_empty() {
            self.push_entry(Dir::Rx, raw);
        }
        self.finish_modbus(ExchangeState::Err, reason.to_string());
    }

    pub(super) fn stop_modbus(&mut self, reason: &str) {
        self.abort_modbus(reason);
        self.modbus.polling = false;
    }

    pub(super) fn clear_modbus(&mut self) {
        self.abort_modbus("已清空");
        self.modbus.results.clear();
        self.modbus.rx_buf.clear();
    }

    pub(super) fn modbus_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(sent_at) = self.modbus.pending.as_ref().map(|p| p.sent_at)
            && sent_at.elapsed() >= MODBUS_TIMEOUT
        {
            let raw = std::mem::take(&mut self.modbus.rx_buf);
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
        if self.modbus.polling && self.modbus.pending.is_none() && self.conn.is_some() {
            match self.modbus.interval(cx) {
                Ok(interval) => {
                    if self
                        .modbus
                        .last_sent
                        .is_none_or(|t| t.elapsed() >= interval)
                    {
                        changed |= self.send_modbus_now(cx);
                    }
                }
                Err(e) => {
                    self.modbus.polling = false;
                    self.set_status(e, true);
                    changed = true;
                }
            }
        }
        changed
    }

    fn toggle_modbus_poll(&mut self, cx: &mut Context<Self>) {
        if self.modbus.polling {
            self.modbus.polling = false;
            self.set_status("轮询已停止", false);
        } else if self.conn.is_none() {
            self.set_status("请先打开串口", true);
        } else if let Err(e) = self.modbus.interval(cx) {
            self.set_status(e, true);
        } else if let Err(e) = self.modbus.build_request(cx) {
            self.set_status(e, true);
        } else {
            self.modbus.polling = true;
            self.modbus.last_sent = None;
            self.send_modbus_now(cx);
        }
        self.save(cx);
        cx.notify();
    }

    pub(super) fn modbus_export(&self) -> String {
        let mut out = String::new();
        for ex in &self.modbus.results {
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
}
