use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, StopBits};

pub const BAUD_RATES: &[u32] = &[
    300, 600, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
    1000000, 2000000,
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ParityOpt {
    None,
    Odd,
    Even,
}

impl ParityOpt {
    pub const ALL: [ParityOpt; 3] = [ParityOpt::None, ParityOpt::Odd, ParityOpt::Even];
    pub fn label(self) -> &'static str {
        match self {
            ParityOpt::None => "无 (None)",
            ParityOpt::Odd => "奇校验 (Odd)",
            ParityOpt::Even => "偶校验 (Even)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum FlowOpt {
    None,
    Software,
    Hardware,
}

impl FlowOpt {
    pub const ALL: [FlowOpt; 3] = [FlowOpt::None, FlowOpt::Software, FlowOpt::Hardware];
    pub fn label(self) -> &'static str {
        match self {
            FlowOpt::None => "无",
            FlowOpt::Software => "软件 (XON/XOFF)",
            FlowOpt::Hardware => "硬件 (RTS/CTS)",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct PortSettings {
    pub port_name: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: ParityOpt,
    pub flow: FlowOpt,
}

impl Default for PortSettings {
    fn default() -> Self {
        Self {
            port_name: String::new(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: ParityOpt::None,
            flow: FlowOpt::None,
        }
    }
}

impl PortSettings {
    pub fn data_bits(&self) -> DataBits {
        match self.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        }
    }
    pub fn stop_bits(&self) -> StopBits {
        if self.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        }
    }
    pub fn parity(&self) -> Parity {
        match self.parity {
            ParityOpt::None => Parity::None,
            ParityOpt::Odd => Parity::Odd,
            ParityOpt::Even => Parity::Even,
        }
    }
    pub fn flow(&self) -> FlowControl {
        match self.flow {
            FlowOpt::None => FlowControl::None,
            FlowOpt::Software => FlowControl::Software,
            FlowOpt::Hardware => FlowControl::Hardware,
        }
    }
}
