//! A small VT100/ANSI-style terminal buffer used by the serial terminal mode.
//!
//! Supported: UTF-8 text, `\n` `\r` backspace tab bell, and CSI sequences for
//! cursor movement (`A B C D G H f`), erase (`J K`), insert/delete char (`@ P`),
//! save/restore cursor (`s u`) and 16-colour SGR (`m`). Everything else is skipped.
//! The buffer is an unbounded-width list of lines (no wrapping) with scrollback.

const MAX_LINES: usize = 5000;
/// Assumed screen height for absolute cursor addressing (`ESC [ r ; c H`).
const SCREEN_ROWS: usize = 24;

const PALETTE: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, // normal
    0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff, // bright
];

#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<u32>,
}

const BLANK: Cell = Cell { ch: ' ', fg: None };

#[derive(Clone, Copy, PartialEq)]
enum State {
    Normal,
    Esc,
    Csi,
    /// OSC / DCS etc.: skipped until BEL or ESC.
    Osc,
}

pub struct Terminal {
    lines: Vec<Vec<Cell>>,
    row: usize,
    col: usize,
    saved: (usize, usize),
    fg: Option<u32>,
    state: State,
    csi: String,
    utf8: Vec<u8>,
    /// Treat a received LF as CR+LF (like PuTTY's "implicit CR in every LF").
    pub lf_implies_cr: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            row: 0,
            col: 0,
            saved: (0, 0),
            fg: None,
            state: State::Normal,
            csi: String::new(),
            utf8: Vec::new(),
            lf_implies_cr: true,
        }
    }

    pub fn clear(&mut self) {
        let lf = self.lf_implies_cr;
        *self = Self::new();
        self.lf_implies_cr = lf;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Coloured runs of line `i`; the cursor cell is drawn as a block when `cursor` is set.
    pub fn runs(&self, i: usize, cursor: bool) -> Vec<(String, Option<u32>)> {
        let mut cells = self.lines[i].clone();
        if cursor && i == self.row {
            while cells.len() <= self.col {
                cells.push(BLANK);
            }
            cells[self.col] = Cell { ch: '█', fg: Some(0xcccccc) };
        }
        let mut runs: Vec<(String, Option<u32>)> = Vec::new();
        for c in cells {
            match runs.last_mut() {
                Some((s, fg)) if *fg == c.fg => s.push(c.ch),
                _ => runs.push((c.ch.to_string(), c.fg)),
            }
        }
        runs
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            self.feed_byte(b);
        }
        if self.lines.len() > MAX_LINES {
            let n = self.lines.len() - MAX_LINES;
            self.lines.drain(..n);
            self.row = self.row.saturating_sub(n);
            self.saved.0 = self.saved.0.saturating_sub(n);
        }
    }

    fn feed_byte(&mut self, b: u8) {
        match self.state {
            State::Esc => {
                self.state = match b {
                    b'[' => {
                        self.csi.clear();
                        State::Csi
                    }
                    b']' | b'P' | b'_' | b'^' => State::Osc,
                    b'7' => {
                        self.saved = (self.row, self.col);
                        State::Normal
                    }
                    b'8' => {
                        self.restore_cursor();
                        State::Normal
                    }
                    b'c' => {
                        self.clear();
                        State::Normal
                    }
                    _ => State::Normal,
                };
            }
            State::Csi => {
                if (0x40..=0x7E).contains(&b) {
                    self.finish_csi(b as char);
                    self.state = State::Normal;
                } else {
                    self.csi.push(b as char);
                }
            }
            State::Osc => match b {
                0x07 => self.state = State::Normal,
                0x1B => self.state = State::Esc,
                _ => {}
            },
            State::Normal => self.normal_byte(b),
        }
    }

    fn params(&self) -> Vec<usize> {
        self.csi
            .trim_start_matches('?')
            .split(';')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }

    fn restore_cursor(&mut self) {
        self.row = self.saved.0.min(self.lines.len() - 1);
        self.col = self.saved.1;
    }

    fn ensure_row(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    fn finish_csi(&mut self, fin: char) {
        if self.csi.starts_with('?') || self.csi.starts_with('>') {
            return; // private modes: ignored
        }
        let p = self.params();
        let n = |i: usize| p.get(i).copied().filter(|&v| v > 0).unwrap_or(1);
        match fin {
            'A' => self.row = self.row.saturating_sub(n(0)),
            'B' => {
                self.row += n(0);
                self.ensure_row(self.row);
            }
            'C' => self.col += n(0),
            'D' => self.col = self.col.saturating_sub(n(0)),
            'G' => self.col = n(0) - 1,
            'H' | 'f' => {
                let top = self.lines.len().saturating_sub(SCREEN_ROWS);
                self.row = top + n(0) - 1;
                self.col = n(1) - 1;
                self.ensure_row(self.row);
            }
            'J' => match p[0] {
                0 => {
                    let (r, c) = (self.row, self.col);
                    self.lines[r].truncate(c);
                    self.lines.truncate(r + 1);
                }
                1 => {
                    for l in &mut self.lines[..self.row] {
                        l.clear();
                    }
                    let c = self.col;
                    for cell in self.lines[self.row].iter_mut().take(c + 1) {
                        *cell = BLANK;
                    }
                }
                _ => {
                    // Clear screen: blank the visible window and home the cursor.
                    let top = self.lines.len().saturating_sub(SCREEN_ROWS);
                    for l in &mut self.lines[top..] {
                        l.clear();
                    }
                    self.row = top;
                    self.col = 0;
                }
            },
            'K' => {
                let c = self.col;
                let line = &mut self.lines[self.row];
                match p[0] {
                    0 => line.truncate(c),
                    1 => {
                        for cell in line.iter_mut().take(c + 1) {
                            *cell = BLANK;
                        }
                    }
                    _ => line.clear(),
                }
            }
            '@' => {
                let (c, k) = (self.col, n(0));
                let line = &mut self.lines[self.row];
                while line.len() < c {
                    line.push(BLANK);
                }
                for _ in 0..k {
                    line.insert(c, BLANK);
                }
            }
            'P' => {
                let (c, k) = (self.col, n(0));
                let line = &mut self.lines[self.row];
                if c < line.len() {
                    line.drain(c..(c + k).min(line.len()));
                }
            }
            's' => self.saved = (self.row, self.col),
            'u' => self.restore_cursor(),
            'm' => self.sgr(&p),
            _ => {}
        }
    }

    fn sgr(&mut self, p: &[usize]) {
        let mut i = 0;
        while i < p.len() {
            match p[i] {
                0 | 39 => self.fg = None,
                v @ 30..=37 => self.fg = Some(PALETTE[v - 30]),
                v @ 90..=97 => self.fg = Some(PALETTE[v - 90 + 8]),
                38 => {
                    // 38;5;n (256-colour, 16 base colours only) or 38;2;r;g;b
                    match p.get(i + 1) {
                        Some(5) => {
                            if let Some(&n) = p.get(i + 2) {
                                self.fg = Some(xterm256(n));
                            }
                            i += 2;
                        }
                        Some(2) => {
                            if let (Some(&r), Some(&g), Some(&b)) =
                                (p.get(i + 2), p.get(i + 3), p.get(i + 4))
                            {
                                self.fg = Some(((r as u32 & 255) << 16) | ((g as u32 & 255) << 8) | (b as u32 & 255));
                            }
                            i += 4;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn normal_byte(&mut self, b: u8) {
        if !self.utf8.is_empty() || b >= 0x80 {
            self.utf8.push(b);
            match std::str::from_utf8(&self.utf8) {
                Ok(s) => {
                    let ch = s.chars().next().unwrap();
                    self.utf8.clear();
                    self.put(ch);
                }
                Err(e) if e.error_len().is_none() && self.utf8.len() < 4 => {} // incomplete
                Err(_) => {
                    self.utf8.clear();
                    self.put('\u{FFFD}');
                }
            }
            return;
        }
        match b {
            b'\n' | 0x0B | 0x0C => {
                self.row += 1;
                self.ensure_row(self.row);
                if self.lf_implies_cr {
                    self.col = 0;
                }
            }
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => self.col = (self.col / 8 + 1) * 8,
            0x1B => self.state = State::Esc,
            0x20..=0x7E => self.put(b as char),
            _ => {}
        }
    }

    fn put(&mut self, ch: char) {
        let (col, fg) = (self.col, self.fg);
        let line = &mut self.lines[self.row];
        while line.len() < col {
            line.push(BLANK);
        }
        let cell = Cell { ch, fg };
        if col < line.len() {
            line[col] = cell;
        } else {
            line.push(cell);
        }
        self.col += 1;
    }
}

fn xterm256(n: usize) -> u32 {
    match n {
        0..=15 => PALETTE[n],
        16..=231 => {
            let n = n - 16;
            let lv = |v: usize| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            (lv(n / 36) << 16) | (lv(n / 6 % 6) << 8) | lv(n % 6)
        }
        _ => {
            let g = 8 + 10 * (n.min(255) - 232) as u32;
            (g << 16) | (g << 8) | g
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_cr() {
        let mut t = Terminal::new();
        t.feed(b"hello\r\nworld");
        assert_eq!(t.text(), "hello\nworld");
        t.feed(b"\rWO");
        assert_eq!(t.text(), "hello\nWOrld");
    }

    #[test]
    fn bare_lf_option() {
        let mut t = Terminal::new();
        t.lf_implies_cr = false;
        t.feed(b"ab\ncd");
        assert_eq!(t.text(), "ab\n  cd");
    }

    #[test]
    fn backspace_erase() {
        let mut t = Terminal::new();
        t.feed(b"abc\x08 \x08");
        assert_eq!(t.text(), "ab");
        t.feed(b"\x1b[K");
        assert_eq!(t.text(), "ab");
        t.feed(b"\x08\x1b[K");
        assert_eq!(t.text(), "a");
    }

    #[test]
    fn cursor_movement() {
        let mut t = Terminal::new();
        t.feed(b"abcdef\x1b[3D\x1b[K");
        assert_eq!(t.text(), "abc");
        t.feed(b"\x1b[2Dxy"); // back 2 -> col 1, overwrite
        assert_eq!(t.text(), "axy");
        t.feed(b"\x1b[H1"); // home
        assert_eq!(t.text(), "1xy");
        t.feed(b"\x1b[2J");
        assert_eq!(t.text(), "");
    }

    #[test]
    fn insert_delete_char() {
        let mut t = Terminal::new();
        t.feed(b"abcd\x1b[3D\x1b[P");
        assert_eq!(t.text(), "acd");
        t.feed(b"\x1b[@X");
        assert_eq!(t.text(), "aXcd");
    }

    #[test]
    fn colors_and_skipped_escapes() {
        let mut t = Terminal::new();
        t.feed(b"\x1b[31mred\x1b[0m ok\x1b]0;title\x07!");
        assert_eq!(t.text(), "red ok!");
        let runs = t.runs(0, false);
        assert_eq!(runs[0], ("red".to_string(), Some(0xcd3131)));
        assert_eq!(runs[1].1, None);
    }

    #[test]
    fn utf8_split_across_feeds() {
        let mut t = Terminal::new();
        let b = "中".as_bytes();
        t.feed(&b[..1]);
        t.feed(&b[1..]);
        assert_eq!(t.text(), "中");
    }
}
