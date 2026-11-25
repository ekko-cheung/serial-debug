mod commands;
mod connection;
mod log;
mod modbus_master;
mod send;
mod terminal;
mod ui;

use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Subscription, Task,
    UniformListScrollHandle, Window,
};

use crate::model::{DataMode, LogEntry, Row};
use crate::serial::config::PortSettings;
use crate::serial::worker::SerialConn;
use crate::storage::{self, AppConfig, BUILTIN_COUNT, SavedCommand, Suffix};
use crate::terminal::Terminal;
use crate::text_input::{Submitted, TextInput};

use self::modbus_master::ModbusMaster;
use self::ui::Dropdown;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Debug,
    Terminal,
    Modbus,
}

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

    modbus: ModbusMaster,

    custom_suffixes: Vec<Suffix>,
    suffix_idx: usize,
    show_add_suffix: bool,
    suffix_name: Entity<TextInput>,
    suffix_text: Entity<TextInput>,
    send_input: Entity<TextInput>,
    saved_commands: Vec<SavedCommand>,

    open_dropdown: Option<Dropdown>,
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

        let modbus = ModbusMaster::new(cx, &cfg.modbus);
        let mut subs = vec![
            cx.subscribe(&custom_baud, |this, _, _: &Submitted, cx| {
                this.apply_custom_baud(cx)
            }),
            cx.subscribe(&send_input, |this, _, _: &Submitted, cx| this.send(cx)),
        ];
        subs.extend(modbus.subscribe(cx));

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
            modbus,
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
            saved_commands: cfg.saved_commands,
            open_dropdown: None,
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

    fn set_status(&mut self, msg: impl Into<String>, is_err: bool) {
        self.status = Some((msg.into(), is_err));
    }

    fn save(&self, cx: &App) {
        storage::save(&AppConfig {
            settings: self.settings.clone(),
            custom_suffixes: self.custom_suffixes.clone(),
            saved_commands: self.saved_commands.clone(),
            suffix_idx: self.suffix_idx,
            tx_mode: self.tx_mode,
            rx_mode: self.rx_mode,
            show_ts: self.show_ts,
            auto_scroll: self.auto_scroll,
            modbus: self.modbus.config(cx),
        });
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        if self.mode == Mode::Modbus {
            self.stop_modbus("已切换到其他模式");
        }
        self.mode = mode;
        self.open_dropdown = None;
        let focus = match mode {
            Mode::Terminal => self.term_focus.clone(),
            Mode::Debug => self.send_input.read(cx).focus_handle(cx),
            Mode::Modbus => self.modbus.focus_handle(cx),
        };
        window.focus(&focus);
        self.rebuild_rows();
        self.scroll_to_end();
        self.save(cx);
        cx.notify();
    }
}
