use std::ops::Range;

use gpui::{Context, Div, MouseButton, Window, div, prelude::*, px, rgb, uniform_list};

use super::SerialApp;
use super::ui::{BORDER, TEXT};

/// Bytes sent by the Enter key in terminal mode: CR, LF, CRLF.
const EOL_BYTES: [&[u8]; 3] = [&[0x0d], &[0x0a], &[0x0d, 0x0a]];
pub(super) const EOL_LABELS: [&str; 3] = ["CR", "LF", "CR+LF"];

impl SerialApp {
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

    pub(super) fn terminal_view(&self, cx: &mut Context<Self>) -> Div {
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
}
