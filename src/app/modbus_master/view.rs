use std::ops::Range;

use gpui::{ClickEvent, Context, Div, Window, div, prelude::*, px, rgb, uniform_list};

use crate::modbus::{ByteOrder, Decode, Fc};

use super::super::ui::{BORDER, Dropdown, GREEN, MUTED, ORANGE, PANEL, RED};
use super::{ExchangeState, SerialApp};

impl SerialApp {
    pub(in crate::app) fn modbus_results_view(&self, cx: &mut Context<Self>) -> Div {
        let count = self.modbus.results.len();
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
                                this.modbus.results.clear();
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
                                        let ex = &this.modbus.results[ix];
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
                        .track_scroll(self.modbus.result_scroll.clone())
                        .size_full(),
                    ),
            )
    }

    pub(in crate::app) fn modbus_panel(&self, cx: &Context<Self>) -> Div {
        let fc = self.modbus.fc;
        let err = self.modbus.build_request(cx).err();
        let connected = self.conn.is_some();

        let fc_idx = Fc::ALL.iter().position(|f| *f == fc).unwrap_or(2);

        let mut row = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_2()
            .child(Self::field(
                "从站",
                div().w(px(64.)).child(self.modbus.slave.clone()),
            ))
            .child(Self::field(
                "功能码",
                div().w(px(170.)).child(self.dropdown(
                    Dropdown::ModbusFc,
                    Fc::ALL[fc_idx].label().to_string(),
                    Fc::ALL.iter().map(|f| f.label().to_string()).collect(),
                    |this, i, _| this.modbus.fc = Fc::ALL[i],
                    cx,
                )),
            ))
            .child(Self::field(
                "起始地址",
                div().w(px(96.)).child(self.modbus.start.clone()),
            ));

        if fc.is_read() {
            row = row.child(Self::field(
                "数量",
                div().w(px(76.)).child(self.modbus.qty.clone()),
            ));
        } else if fc == Fc::WriteSingleCoil {
            row = row.child(Self::field(
                "值",
                self.seg(
                    "mb-coil",
                    ("ON", self.modbus.coil_on),
                    ("OFF", !self.modbus.coil_on),
                    |this, cx| {
                        this.modbus.coil_on = true;
                        this.save(cx);
                        cx.notify();
                    },
                    |this, cx| {
                        this.modbus.coil_on = false;
                        this.save(cx);
                        cx.notify();
                    },
                    cx,
                ),
            ));
        } else if fc == Fc::WriteSingleRegister {
            row = row.child(Self::field(
                "值",
                div().w(px(96.)).child(self.modbus.value.clone()),
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
                            if self.modbus.polling {
                                "停止轮询"
                            } else {
                                "周期轮询"
                            },
                            self.modbus.polling,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_modbus_poll(cx)),
                        ),
                    ),
            ))
            .child(Self::field(
                "间隔 ms",
                div().w(px(76.)).child(self.modbus.poll_interval.clone()),
            ));

        let mut extra = div().flex().flex_wrap().items_end().gap_3();
        let has_extra = match fc {
            Fc::WriteMultipleCoils => {
                extra = extra.child(div().flex_1().min_w_0().child(Self::field(
                    "线圈值 (0/1 序列，如 1 0 1 1)",
                    self.modbus.values.clone(),
                )));
                true
            }
            Fc::WriteMultipleRegisters => {
                extra = extra.child(div().flex_1().min_w_0().child(Self::field(
                    "寄存器值 (十进制或 0x，如 0 100 0x1F)",
                    self.modbus.values.clone(),
                )));
                true
            }
            Fc::ReadHoldingRegisters | Fc::ReadInputRegisters => {
                extra = extra
                    .child(Self::field(
                        "解码",
                        div().w(px(104.)).child(self.dropdown(
                            Dropdown::ModbusDecode,
                            self.modbus.decode.label().to_string(),
                            Decode::ALL.iter().map(|d| d.label().to_string()).collect(),
                            |this, i, cx| {
                                this.modbus.decode = Decode::ALL[i];
                                this.save(cx);
                            },
                            cx,
                        )),
                    ))
                    .when(self.modbus.decode.is_32bit(), |d| {
                        d.child(Self::field(
                            "字节序",
                            div().w(px(150.)).child(
                                self.dropdown(
                                    Dropdown::ModbusOrder,
                                    self.modbus.byte_order.label().to_string(),
                                    ByteOrder::ALL
                                        .iter()
                                        .map(|o| o.label().to_string())
                                        .collect(),
                                    |this, i, cx| {
                                        this.modbus.byte_order = ByteOrder::ALL[i];
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
}
