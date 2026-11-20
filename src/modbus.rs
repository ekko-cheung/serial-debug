//! Modbus RTU master support: request framing, CRC-16, response parsing and
//! register decoding. The UI lives in `app.rs`; this module is GPUI-free.

use serde::{Deserialize, Serialize};

pub const MAX_READ_COILS: u16 = 2000;
pub const MAX_READ_REGISTERS: u16 = 125;
pub const MAX_WRITE_COILS: usize = 1968;
pub const MAX_WRITE_REGISTERS: usize = 123;

/// Function codes supported by the request form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fc {
    ReadCoils,
    ReadDiscreteInputs,
    ReadHoldingRegisters,
    ReadInputRegisters,
    WriteSingleCoil,
    WriteSingleRegister,
    WriteMultipleCoils,
    WriteMultipleRegisters,
}

impl Fc {
    pub const ALL: [Fc; 8] = [
        Fc::ReadCoils,
        Fc::ReadDiscreteInputs,
        Fc::ReadHoldingRegisters,
        Fc::ReadInputRegisters,
        Fc::WriteSingleCoil,
        Fc::WriteSingleRegister,
        Fc::WriteMultipleCoils,
        Fc::WriteMultipleRegisters,
    ];

    pub fn code(self) -> u8 {
        match self {
            Fc::ReadCoils => 0x01,
            Fc::ReadDiscreteInputs => 0x02,
            Fc::ReadHoldingRegisters => 0x03,
            Fc::ReadInputRegisters => 0x04,
            Fc::WriteSingleCoil => 0x05,
            Fc::WriteSingleRegister => 0x06,
            Fc::WriteMultipleCoils => 0x0f,
            Fc::WriteMultipleRegisters => 0x10,
        }
    }

    pub fn from_code(code: u8) -> Option<Fc> {
        Fc::ALL.into_iter().find(|fc| fc.code() == code)
    }

    pub fn label(self) -> &'static str {
        match self {
            Fc::ReadCoils => "01 读线圈",
            Fc::ReadDiscreteInputs => "02 读离散输入",
            Fc::ReadHoldingRegisters => "03 读保持寄存器",
            Fc::ReadInputRegisters => "04 读输入寄存器",
            Fc::WriteSingleCoil => "05 写单线圈",
            Fc::WriteSingleRegister => "06 写单寄存器",
            Fc::WriteMultipleCoils => "0F 写多线圈",
            Fc::WriteMultipleRegisters => "10 写多寄存器",
        }
    }

    pub fn is_read(self) -> bool {
        matches!(
            self,
            Fc::ReadCoils
                | Fc::ReadDiscreteInputs
                | Fc::ReadHoldingRegisters
                | Fc::ReadInputRegisters
        )
    }

    pub fn is_coil(self) -> bool {
        matches!(
            self,
            Fc::ReadCoils | Fc::ReadDiscreteInputs | Fc::WriteSingleCoil | Fc::WriteMultipleCoils
        )
    }
}

/// Payload that distinguishes the write function codes.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestData {
    Read,
    SingleCoil(bool),
    SingleRegister(u16),
    Coils(Vec<bool>),
    Registers(Vec<u16>),
}

#[derive(Clone, Debug)]
pub struct Request {
    pub slave: u8,
    pub fc: Fc,
    pub address: u16,
    pub quantity: u16,
    pub data: RequestData,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.slave > 247 {
            return Err(format!("从站地址必须在 0..=247，收到 {}", self.slave));
        }
        if self.address as u32 + self.quantity.max(1) as u32 > 0x1_0000 {
            return Err("起始地址 + 数量超出 0..=65535".into());
        }
        if self.fc.is_read() {
            if self.slave == 0 {
                return Err("广播地址不能用于读操作".into());
            }
            let max = if self.fc.is_coil() {
                MAX_READ_COILS
            } else {
                MAX_READ_REGISTERS
            };
            if self.quantity == 0 || self.quantity > max {
                return Err(format!("数量必须在 1..={max}"));
            }
            if !matches!(self.data, RequestData::Read) {
                return Err("请求数据与功能码不匹配".into());
            }
            return Ok(());
        }

        match (&self.fc, &self.data) {
            (Fc::WriteSingleCoil, RequestData::SingleCoil(_)) => {
                if self.quantity != 1 {
                    return Err("写单线圈的数量必须为 1".into());
                }
            }
            (Fc::WriteSingleRegister, RequestData::SingleRegister(_)) => {
                if self.quantity != 1 {
                    return Err("写单寄存器的数量必须为 1".into());
                }
            }
            (Fc::WriteMultipleCoils, RequestData::Coils(values)) => {
                if values.is_empty() || values.len() > MAX_WRITE_COILS {
                    return Err(format!("线圈数量必须在 1..={MAX_WRITE_COILS}"));
                }
                if self.quantity as usize != values.len() {
                    return Err("线圈数量与值列表长度不一致".into());
                }
            }
            (Fc::WriteMultipleRegisters, RequestData::Registers(values)) => {
                if values.is_empty() || values.len() > MAX_WRITE_REGISTERS {
                    return Err(format!("寄存器数量必须在 1..={MAX_WRITE_REGISTERS}"));
                }
                if self.quantity as usize != values.len() {
                    return Err("寄存器数量与值列表长度不一致".into());
                }
            }
            _ => return Err("请求数据与功能码不匹配".into()),
        }
        Ok(())
    }

    /// Build the PDU (function code + data, without slave address and CRC).
    pub fn pdu(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut pdu = vec![self.fc.code()];
        let addr = self.address.to_be_bytes();
        match &self.data {
            RequestData::Read => {
                pdu.extend_from_slice(&addr);
                pdu.extend_from_slice(&self.quantity.to_be_bytes());
            }
            RequestData::SingleCoil(on) => {
                pdu.extend_from_slice(&addr);
                pdu.extend_from_slice(&if *on { 0xff00u16 } else { 0 }.to_be_bytes());
            }
            RequestData::SingleRegister(value) => {
                pdu.extend_from_slice(&addr);
                pdu.extend_from_slice(&value.to_be_bytes());
            }
            RequestData::Coils(values) => {
                pdu.extend_from_slice(&addr);
                pdu.extend_from_slice(&self.quantity.to_be_bytes());
                let byte_count = values.len().div_ceil(8);
                pdu.push(byte_count as u8);
                let mut packed = vec![0u8; byte_count];
                for (i, on) in values.iter().enumerate() {
                    if *on {
                        packed[i / 8] |= 1 << (i % 8);
                    }
                }
                pdu.extend_from_slice(&packed);
            }
            RequestData::Registers(values) => {
                pdu.extend_from_slice(&addr);
                pdu.extend_from_slice(&self.quantity.to_be_bytes());
                pdu.push((values.len() * 2) as u8);
                for v in values {
                    pdu.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
        Ok(pdu)
    }

    /// Build the full RTU frame (slave + PDU + CRC).
    pub fn adu(&self) -> Result<Vec<u8>, String> {
        let mut adu = vec![self.slave];
        adu.extend_from_slice(&self.pdu()?);
        let crc = crc16(&adu);
        adu.push(crc as u8);
        adu.push((crc >> 8) as u8);
        Ok(adu)
    }
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xa001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn check_crc(frame: &[u8]) -> Result<(), String> {
    if frame.len() < 3 {
        return Err("响应帧过短".into());
    }
    let (body, crc_bytes) = frame.split_at(frame.len() - 2);
    let received = u16::from_le_bytes([crc_bytes[0], crc_bytes[1]]);
    let computed = crc16(body);
    if received != computed {
        return Err(format!(
            "CRC 校验失败: 计算值 0x{computed:04X}, 接收到 0x{received:04X}"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResponseValues {
    Bits(Vec<bool>),
    Registers(Vec<u16>),
    WriteCoil { address: u16, on: bool },
    WriteRegister { address: u16, value: u16 },
    WriteCoils { address: u16, quantity: u16 },
    WriteRegisters { address: u16, quantity: u16 },
}

#[derive(Clone, Debug)]
pub struct Response {
    pub values: ResponseValues,
    pub raw: Vec<u8>,
}

/// Result of trying to parse the bytes buffered so far.
#[derive(Clone, Debug)]
pub enum Progress {
    /// The frame is not complete yet; keep reading.
    NeedMore,
    Ready(Response),
    /// The slave answered with a Modbus exception.
    Exception {
        code: u8,
        raw: Vec<u8>,
    },
    /// The buffered bytes cannot be a response to this request.
    Invalid(String),
}

pub fn try_parse_response(buf: &[u8], req: &Request) -> Progress {
    if buf.is_empty() {
        return Progress::NeedMore;
    }
    if buf.len() > 300 {
        return Progress::Invalid("响应数据过长".into());
    }
    if buf[0] != req.slave {
        return Progress::Invalid(format!(
            "从站地址不匹配: 期望 {}, 收到 {}",
            req.slave, buf[0]
        ));
    }
    if buf.len() < 2 {
        return Progress::NeedMore;
    }
    let expected_fc = req.fc.code();
    let received_fc = buf[1];
    if received_fc == expected_fc | 0x80 {
        if buf.len() < 5 {
            return Progress::NeedMore;
        }
        let frame = &buf[..5];
        if let Err(e) = check_crc(frame) {
            return Progress::Invalid(e);
        }
        return Progress::Exception {
            code: frame[2],
            raw: frame.to_vec(),
        };
    }
    if received_fc != expected_fc {
        return Progress::Invalid(format!(
            "功能码不匹配: 期望 0x{expected_fc:02X}, 收到 0x{received_fc:02X}"
        ));
    }

    if req.fc.is_read() {
        if buf.len() < 3 {
            return Progress::NeedMore;
        }
        let byte_count = buf[2] as usize;
        let expected_bytes = if req.fc.is_coil() {
            (req.quantity as usize).div_ceil(8)
        } else {
            req.quantity as usize * 2
        };
        if byte_count != expected_bytes {
            return Progress::Invalid(format!(
                "字节数不匹配: 期望 {expected_bytes}, 收到 {byte_count}"
            ));
        }
        let frame_len = 3 + byte_count + 2;
        if buf.len() < frame_len {
            return Progress::NeedMore;
        }
        let frame = &buf[..frame_len];
        if let Err(e) = check_crc(frame) {
            return Progress::Invalid(e);
        }
        let values = if req.fc.is_coil() {
            let mut bits = Vec::with_capacity(req.quantity as usize);
            'outer: for &byte in &frame[3..3 + byte_count] {
                for bit in 0..8 {
                    if bits.len() == req.quantity as usize {
                        break 'outer;
                    }
                    bits.push((byte >> bit) & 1 == 1);
                }
            }
            ResponseValues::Bits(bits)
        } else {
            let mut regs = Vec::with_capacity(req.quantity as usize);
            for i in 0..req.quantity as usize {
                let hi = frame[3 + i * 2];
                let lo = frame[4 + i * 2];
                regs.push(u16::from_be_bytes([hi, lo]));
            }
            ResponseValues::Registers(regs)
        };
        return Progress::Ready(Response {
            values,
            raw: frame.to_vec(),
        });
    }

    // Write responses echo the request's address and value/quantity.
    let frame_len = 8;
    if buf.len() < frame_len {
        return Progress::NeedMore;
    }
    let frame = &buf[..frame_len];
    if let Err(e) = check_crc(frame) {
        return Progress::Invalid(e);
    }
    let address = u16::from_be_bytes([frame[2], frame[3]]);
    if address != req.address {
        return Progress::Invalid(format!(
            "响应地址回显不匹配: 期望 {}, 收到 {address}",
            req.address
        ));
    }
    let echo = u16::from_be_bytes([frame[4], frame[5]]);
    let values = match (&req.fc, &req.data) {
        (Fc::WriteSingleCoil, RequestData::SingleCoil(on)) => {
            let expected = if *on { 0xff00 } else { 0 };
            if echo != expected {
                return Progress::Invalid(format!(
                    "线圈值回显不匹配: 期望 0x{expected:04X}, 收到 0x{echo:04X}"
                ));
            }
            ResponseValues::WriteCoil {
                address,
                on: echo == 0xff00,
            }
        }
        (Fc::WriteSingleRegister, RequestData::SingleRegister(value)) => {
            if echo != *value {
                return Progress::Invalid(format!("寄存器值回显不匹配: 期望 {value}, 收到 {echo}"));
            }
            ResponseValues::WriteRegister {
                address,
                value: echo,
            }
        }
        (Fc::WriteMultipleCoils, _) | (Fc::WriteMultipleRegisters, _) => {
            if echo != req.quantity {
                return Progress::Invalid(format!(
                    "数量回显不匹配: 期望 {}, 收到 {echo}",
                    req.quantity
                ));
            }
            if req.fc == Fc::WriteMultipleCoils {
                ResponseValues::WriteCoils {
                    address,
                    quantity: echo,
                }
            } else {
                ResponseValues::WriteRegisters {
                    address,
                    quantity: echo,
                }
            }
        }
        _ => return Progress::Invalid("请求数据与功能码不匹配".into()),
    };
    Progress::Ready(Response {
        values,
        raw: frame.to_vec(),
    })
}

pub fn exception_name(code: u8) -> &'static str {
    match code {
        0x01 => "非法功能码",
        0x02 => "非法数据地址",
        0x03 => "非法数据值",
        0x04 => "从站设备故障",
        0x05 => "确认（处理中）",
        0x06 => "从站设备忙",
        0x08 => "存储奇偶校验错误",
        0x0a => "网关路径不可用",
        0x0b => "网关目标设备无响应",
        _ => "未知异常码",
    }
}

pub fn describe_request(req: &Request) -> String {
    let head = format!("[{}] {}", req.slave, req.fc.label());
    match &req.data {
        RequestData::Read => format!("{head} 地址 {} 数量 {}", req.address, req.quantity),
        RequestData::SingleCoil(on) => {
            format!(
                "{head} 地址 {} = {}",
                req.address,
                if *on { "ON" } else { "OFF" }
            )
        }
        RequestData::SingleRegister(value) => {
            format!("{head} 地址 {} = {} (0x{value:04X})", req.address, value)
        }
        RequestData::Coils(values) => {
            format!("{head} 地址 {} 数量 {}", req.address, values.len())
        }
        RequestData::Registers(values) => {
            format!("{head} 地址 {} 数量 {}", req.address, values.len())
        }
    }
}

pub fn describe_response(resp: &Response, decode: Decode, order: ByteOrder) -> String {
    match &resp.values {
        ResponseValues::Bits(bits) => format!("线圈: {}", format_bits(bits, 64)),
        ResponseValues::Registers(regs) => {
            let mut text = format!("寄存器: {}", format_registers(regs, 24));
            if decode != Decode::Raw {
                let decoded = decode_registers(regs, decode, order);
                if !decoded.is_empty() {
                    let shown = decoded
                        .iter()
                        .take(16)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    let suffix = if decoded.len() > 16 { ", …" } else { "" };
                    text.push_str(&format!(" | {}: {shown}{suffix}", decode.label()));
                }
            }
            text
        }
        ResponseValues::WriteCoil { address, on } => {
            format!("线圈 {address} 已写入 {}", if *on { "ON" } else { "OFF" })
        }
        ResponseValues::WriteRegister { address, value } => {
            format!("寄存器 {address} 已写入 {value} (0x{value:04X})")
        }
        ResponseValues::WriteCoils { address, quantity } => {
            format!("线圈 {address} 起 {quantity} 个已写入")
        }
        ResponseValues::WriteRegisters { address, quantity } => {
            format!("寄存器 {address} 起 {quantity} 个已写入")
        }
    }
}

fn format_bits(bits: &[bool], limit: usize) -> String {
    let shown = bits
        .iter()
        .take(limit)
        .map(|b| if *b { "1" } else { "0" })
        .collect::<Vec<_>>()
        .join(" ");
    if bits.len() > limit {
        format!("{shown} … (共 {} 个)", bits.len())
    } else {
        shown
    }
}

fn format_registers(regs: &[u16], limit: usize) -> String {
    let shown = regs
        .iter()
        .take(limit)
        .map(|r| format!("0x{r:04X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if regs.len() > limit {
        format!("{shown} … (共 {} 个)", regs.len())
    } else {
        shown
    }
}

/// Optional interpretation of 16-bit register values.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Decode {
    #[default]
    Raw,
    U16,
    I16,
    U32,
    I32,
    F32,
}

impl Decode {
    pub const ALL: [Decode; 6] = [
        Decode::Raw,
        Decode::U16,
        Decode::I16,
        Decode::U32,
        Decode::I32,
        Decode::F32,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Decode::Raw => "原始值",
            Decode::U16 => "u16",
            Decode::I16 => "i16",
            Decode::U32 => "u32",
            Decode::I32 => "i32",
            Decode::F32 => "f32",
        }
    }

    pub fn is_32bit(self) -> bool {
        matches!(self, Decode::U32 | Decode::I32 | Decode::F32)
    }
}

/// Byte/word order when combining two 16-bit registers into a 32-bit value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ByteOrder {
    #[default]
    Abcd,
    Cdab,
    Badc,
    Dcba,
}

impl ByteOrder {
    pub const ALL: [ByteOrder; 4] = [
        ByteOrder::Abcd,
        ByteOrder::Cdab,
        ByteOrder::Badc,
        ByteOrder::Dcba,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ByteOrder::Abcd => "ABCD (大端)",
            ByteOrder::Cdab => "CDAB (字交换)",
            ByteOrder::Badc => "BADC (字节交换)",
            ByteOrder::Dcba => "DCBA (小端)",
        }
    }

    fn apply(self, a: u16, b: u16) -> [u8; 4] {
        let ab = a.to_be_bytes();
        let cd = b.to_be_bytes();
        match self {
            ByteOrder::Abcd => [ab[0], ab[1], cd[0], cd[1]],
            ByteOrder::Cdab => [cd[0], cd[1], ab[0], ab[1]],
            ByteOrder::Badc => [ab[1], ab[0], cd[1], cd[0]],
            ByteOrder::Dcba => [cd[1], cd[0], ab[1], ab[0]],
        }
    }
}

pub fn decode_registers(regs: &[u16], decode: Decode, order: ByteOrder) -> Vec<String> {
    match decode {
        Decode::Raw => Vec::new(),
        Decode::U16 => regs.iter().map(|r| r.to_string()).collect(),
        Decode::I16 => regs.iter().map(|r| (*r as i16).to_string()).collect(),
        Decode::U32 | Decode::I32 | Decode::F32 => regs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let bytes = order.apply(pair[0], pair[1]);
                match decode {
                    Decode::U32 => u32::from_be_bytes(bytes).to_string(),
                    Decode::I32 => i32::from_be_bytes(bytes).to_string(),
                    Decode::F32 => f32::from_be_bytes(bytes).to_string(),
                    _ => unreachable!(),
                }
            })
            .collect(),
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal number.
pub fn parse_number(text: &str) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("不能为空".into());
    }
    let (digits, radix) =
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (hex, 16)
        } else {
            (text, 10)
        };
    u32::from_str_radix(digits, radix).map_err(|_| format!("无效数字: {text}"))
}

pub fn parse_u16_list(text: &str) -> Result<Vec<u16>, String> {
    let mut values = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        if token.is_empty() {
            continue;
        }
        let v = parse_number(token)?;
        if v > u16::MAX as u32 {
            return Err(format!("寄存器值超出 0..=65535: {token}"));
        }
        values.push(v as u16);
    }
    Ok(values)
}

pub fn parse_coil_list(text: &str) -> Result<Vec<bool>, String> {
    let mut values = Vec::new();
    for c in text.chars() {
        if c.is_whitespace() || c == ',' || c == ';' {
            continue;
        }
        match c {
            '0' => values.push(false),
            '1' => values.push(true),
            _ => return Err(format!("线圈值只能由 0/1 组成，非法字符: '{c}'")),
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_crc(mut body: Vec<u8>) -> Vec<u8> {
        let crc = crc16(&body);
        body.push(crc as u8);
        body.push((crc >> 8) as u8);
        body
    }

    fn read_holding(slave: u8, address: u16, quantity: u16) -> Request {
        Request {
            slave,
            fc: Fc::ReadHoldingRegisters,
            address,
            quantity,
            data: RequestData::Read,
        }
    }

    #[test]
    fn crc_known_vector() {
        assert_eq!(crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0a]), 0xcdc5);
    }

    #[test]
    fn build_read_request() {
        let adu = read_holding(1, 0, 10).adu().unwrap();
        assert_eq!(adu, vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x0a, 0xc5, 0xcd]);
    }

    #[test]
    fn parse_register_response() {
        let req = read_holding(1, 0, 2);
        let frame = with_crc(vec![0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x0b]);
        match try_parse_response(&frame, &req) {
            Progress::Ready(resp) => {
                assert_eq!(resp.values, ResponseValues::Registers(vec![10, 11]));
                assert_eq!(resp.raw, frame);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_partial_then_complete() {
        let req = read_holding(1, 0, 2);
        let frame = with_crc(vec![0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x0b]);
        assert!(matches!(
            try_parse_response(&frame[..3], &req),
            Progress::NeedMore
        ));
        assert!(matches!(
            try_parse_response(&frame, &req),
            Progress::Ready(_)
        ));
    }

    #[test]
    fn parse_coil_response() {
        let req = Request {
            slave: 1,
            fc: Fc::ReadCoils,
            address: 0,
            quantity: 8,
            data: RequestData::Read,
        };
        let frame = with_crc(vec![0x01, 0x01, 0x01, 0xd5]);
        match try_parse_response(&frame, &req) {
            Progress::Ready(resp) => assert_eq!(
                resp.values,
                ResponseValues::Bits(vec![true, false, true, false, true, false, true, true])
            ),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_exception() {
        let req = read_holding(1, 0, 2);
        let frame = with_crc(vec![0x01, 0x83, 0x02]);
        match try_parse_response(&frame, &req) {
            Progress::Exception { code, raw } => {
                assert_eq!(code, 2);
                assert_eq!(raw, frame);
                assert_eq!(exception_name(code), "非法数据地址");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_bad_crc() {
        let req = read_holding(1, 0, 2);
        let mut frame = with_crc(vec![0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x0b]);
        *frame.last_mut().unwrap() ^= 0xff;
        assert!(matches!(
            try_parse_response(&frame, &req),
            Progress::Invalid(_)
        ));
    }

    #[test]
    fn write_coils_packing() {
        let req = Request {
            slave: 2,
            fc: Fc::WriteMultipleCoils,
            address: 0x10,
            quantity: 10,
            data: RequestData::Coils(vec![
                true, false, true, true, false, false, true, true, true, false,
            ]),
        };
        let pdu = req.pdu().unwrap();
        assert_eq!(
            pdu,
            vec![0x0f, 0x00, 0x10, 0x00, 0x0a, 0x02, 0b11001101, 0b00000001]
        );
    }

    #[test]
    fn write_coils_response() {
        let req = Request {
            slave: 2,
            fc: Fc::WriteMultipleCoils,
            address: 0x10,
            quantity: 10,
            data: RequestData::Coils(vec![true; 10]),
        };
        let frame = with_crc(vec![0x02, 0x0f, 0x00, 0x10, 0x00, 0x0a]);
        match try_parse_response(&frame, &req) {
            Progress::Ready(resp) => assert_eq!(
                resp.values,
                ResponseValues::WriteCoils {
                    address: 0x10,
                    quantity: 10
                }
            ),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn write_registers_request_and_response() {
        let req = Request {
            slave: 1,
            fc: Fc::WriteMultipleRegisters,
            address: 0x10,
            quantity: 2,
            data: RequestData::Registers(vec![0x000a, 0x0102]),
        };
        let pdu = req.pdu().unwrap();
        assert_eq!(
            pdu,
            vec![0x10, 0x00, 0x10, 0x00, 0x02, 0x04, 0x00, 0x0a, 0x01, 0x02]
        );
        let frame = with_crc(vec![0x01, 0x10, 0x00, 0x10, 0x00, 0x02]);
        match try_parse_response(&frame, &req) {
            Progress::Ready(resp) => assert_eq!(
                resp.values,
                ResponseValues::WriteRegisters {
                    address: 0x10,
                    quantity: 2
                }
            ),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn write_single_register_request_and_response() {
        let req = Request {
            slave: 1,
            fc: Fc::WriteSingleRegister,
            address: 0x10,
            quantity: 1,
            data: RequestData::SingleRegister(0x1234),
        };
        assert_eq!(
            req.adu().unwrap(),
            with_crc(vec![0x01, 0x06, 0x00, 0x10, 0x12, 0x34])
        );
        let frame = with_crc(vec![0x01, 0x06, 0x00, 0x10, 0x12, 0x34]);
        match try_parse_response(&frame, &req) {
            Progress::Ready(resp) => assert_eq!(
                resp.values,
                ResponseValues::WriteRegister {
                    address: 0x10,
                    value: 0x1234
                }
            ),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_byte_orders() {
        let regs = [0x1234, 0x5678];
        assert_eq!(
            decode_registers(&regs, Decode::U32, ByteOrder::Abcd),
            vec!["305419896"]
        );
        assert_eq!(
            decode_registers(&regs, Decode::U32, ByteOrder::Cdab),
            vec!["1450709556"]
        );
        assert_eq!(
            decode_registers(&regs, Decode::U32, ByteOrder::Badc),
            vec!["873625686"]
        );
        assert_eq!(
            decode_registers(&regs, Decode::U32, ByteOrder::Dcba),
            vec!["2018915346"]
        );
        assert_eq!(
            decode_registers(&regs, Decode::I16, ByteOrder::Abcd),
            vec!["4660", "22136"]
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            parse_u16_list("0 1, 0x10; 65535").unwrap(),
            vec![0, 1, 16, 65535]
        );
        assert!(parse_u16_list("65536").is_err());
        assert!(parse_u16_list("abc").is_err());
        assert_eq!(
            parse_coil_list("1 0,1;1").unwrap(),
            vec![true, false, true, true]
        );
        assert!(parse_coil_list("1012").is_err());
    }

    #[test]
    fn validation_bounds() {
        assert!(read_holding(0, 0, 1).validate().is_err());
        assert!(read_holding(1, 0, 0).validate().is_err());
        assert!(read_holding(1, 0, 126).validate().is_err());
        assert!(read_holding(1, 65535, 2).validate().is_err());
        assert!(read_holding(1, 0, 1).validate().is_ok());
    }
}
