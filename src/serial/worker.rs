use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serialport::SerialPort;

use super::config::PortSettings;

pub enum RxMsg {
    Data(Vec<u8>),
    Error(String),
}

/// An open serial connection with a background reader thread.
pub struct SerialConn {
    port: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    pub rx: Receiver<RxMsg>,
    thread: Option<JoinHandle<()>>,
}

impl SerialConn {
    pub fn open(s: &PortSettings) -> Result<Self> {
        if s.port_name.is_empty() {
            return Err(anyhow!("未选择串口"));
        }
        let port = serialport::new(&s.port_name, s.baud_rate)
            .data_bits(s.data_bits())
            .stop_bits(s.stop_bits())
            .parity(s.parity())
            .flow_control(s.flow())
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(|e| anyhow!("打开 {} 失败: {}", s.port_name, e))?;
        let mut reader = port
            .try_clone()
            .map_err(|e| anyhow!("克隆串口失败: {e}"))?;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            let mut pending: Vec<u8> = Vec::new();
            while !stop2.load(Ordering::Relaxed) {
                match reader.read(&mut chunk) {
                    Ok(0) => {}
                    Ok(n) => {
                        pending.extend_from_slice(&chunk[..n]);
                        if pending.len() >= 8192 {
                            let _ = tx.send(RxMsg::Data(std::mem::take(&mut pending)));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                        // Idle gap: flush what we have as one frame.
                        if !pending.is_empty() {
                            let _ = tx.send(RxMsg::Data(std::mem::take(&mut pending)));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        if !pending.is_empty() {
                            let _ = tx.send(RxMsg::Data(std::mem::take(&mut pending)));
                        }
                        let _ = tx.send(RxMsg::Error(format!("读取错误: {e}")));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            port,
            stop,
            rx,
            thread: Some(thread),
        })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        let mut off = 0;
        while off < data.len() {
            match self.port.write(&data[off..]) {
                Ok(0) => return Err(anyhow!("写入失败: 端口返回 0")),
                Ok(n) => off += n,
                // Short write timeout (20ms) at low baud rates: just keep going.
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(anyhow!("写入失败: {e}")),
            }
        }
        self.port.flush().ok();
        Ok(())
    }
}

impl Drop for SerialConn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
