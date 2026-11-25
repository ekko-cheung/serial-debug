use gpui::{ClickEvent, Context, Div, div, prelude::*, px, rgb};

use crate::codec;
use crate::model::DataMode;
use crate::storage::SavedCommand;

use super::SerialApp;
use super::ui::{BORDER, CTRL, MUTED, PANEL};

impl SavedCommand {
    pub(super) fn encode(&self) -> Result<Vec<u8>, String> {
        let mut data = match self.mode {
            DataMode::Ascii => self.text.as_bytes().to_vec(),
            DataMode::Hex => codec::parse_hex(&self.text).map_err(|e| e.to_string())?,
        };
        let suffix = codec::parse_escapes(&self.suffix).map_err(|e| format!("后缀无效: {e}"))?;
        data.extend(suffix);
        Ok(data)
    }
}

impl SerialApp {
    fn save_current_command(&mut self, cx: &mut Context<Self>) {
        let command = self.current_command(cx);
        match command.encode() {
            Err(e) => self.set_status(e, true),
            Ok(data) if data.is_empty() => self.set_status("请输入要保存的命令", true),
            Ok(_) if self.saved_commands.contains(&command) => {
                self.set_status("此命令已保存", false);
            }
            Ok(_) => {
                self.saved_commands.push(command);
                self.save(cx);
                self.set_status("命令已保存", false);
            }
        }
        cx.notify();
    }

    fn send_saved_command(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(command) = self.saved_commands.get(index) {
            let data = command.encode();
            self.send_payload(data, cx);
        }
    }

    fn delete_saved_command(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.saved_commands.len() {
            self.saved_commands.remove(index);
            self.save(cx);
            self.set_status("常用命令已删除", false);
            cx.notify();
        }
    }

    pub(super) fn commands_panel(&self, cx: &Context<Self>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().child("常用命令"))
                    .child(
                        Self::button("save-command", "保存当前命令", false).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.save_current_command(cx)
                            }),
                        ),
                    ),
            )
            .when(self.saved_commands.is_empty(), |d| {
                d.child(div().text_color(rgb(MUTED)).child("暂无常用命令"))
            })
            .when(!self.saved_commands.is_empty(), |d| {
                d.child(
                    div()
                        .id("saved-commands")
                        .max_h(px(112.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(
                            self.saved_commands
                                .iter()
                                .enumerate()
                                .map(|(index, command)| self.command_row(index, command, cx)),
                        ),
                )
            })
    }

    fn command_row(&self, index: usize, command: &SavedCommand, cx: &Context<Self>) -> Div {
        let text = if command.text.is_empty() {
            "（仅后缀）"
        } else {
            command.text.as_str()
        };
        let mode = match command.mode {
            DataMode::Ascii => "ASCII",
            DataMode::Hex => "HEX",
        };

        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(rgb(CTRL))
            .child(div().flex_none().text_color(rgb(MUTED)).child(mode))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family("Consolas")
                    .child(text.to_owned()),
            )
            .when(!command.suffix.is_empty(), |d| {
                d.child(
                    div()
                        .max_w(px(100.))
                        .truncate()
                        .text_color(rgb(MUTED))
                        .child(format!("+ {}", command.suffix)),
                )
            })
            .child(
                Self::button(("send-saved-command", index), "发送", true)
                    .flex_none()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.send_saved_command(index, cx);
                    })),
            )
            .child(
                Self::button(("delete-saved-command", index), "删除", false)
                    .flex_none()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.delete_saved_command(index, cx);
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_commands_encode_with_their_own_format_and_suffix() {
        let ascii = SavedCommand {
            text: "AA 55".into(),
            mode: DataMode::Ascii,
            suffix: "\\r\\n".into(),
        };
        let hex = SavedCommand {
            text: "AA 55".into(),
            mode: DataMode::Hex,
            suffix: "\\x00".into(),
        };

        assert_eq!(ascii.encode().unwrap(), b"AA 55\r\n");
        assert_eq!(hex.encode().unwrap(), [0xAA, 0x55, 0x00]);
    }
}
