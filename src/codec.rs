use anyhow::{anyhow, Result};

/// Parse a hex string such as "AA 55 0x01,ff" into bytes.
pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let mut digits = String::new();
    for token in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        if token.is_empty() {
            continue;
        }
        let t = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        digits.push_str(t);
    }
    if let Some(c) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(anyhow!("非法 HEX 字符: '{c}'"));
    }
    if digits.len() % 2 != 0 {
        return Err(anyhow!("HEX 位数必须为偶数"));
    }
    Ok((0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap())
        .collect())
}

pub fn format_hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lossy UTF-8 display; control characters are shown escaped.
pub fn format_ascii(data: &[u8]) -> String {
    let mut out = String::new();
    for c in String::from_utf8_lossy(data).chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Parse escape sequences (`\r \n \t \0 \\ \xNN`) into bytes. Other text is UTF-8.
pub fn parse_escapes(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match it.next() {
            Some('r') => out.push(b'\r'),
            Some('n') => out.push(b'\n'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('\\') => out.push(b'\\'),
            Some('x') => {
                let h: String = (0..2).filter_map(|_| it.next()).collect();
                if h.len() != 2 {
                    return Err(anyhow!("\\x 后需要两位 HEX"));
                }
                out.push(u8::from_str_radix(&h, 16).map_err(|_| anyhow!("非法 \\x{h}"))?);
            }
            Some(o) => return Err(anyhow!("未知转义 \\{o}")),
            None => return Err(anyhow!("末尾有多余的 \\")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parse() {
        assert_eq!(parse_hex("AA 55 0x01,ff").unwrap(), vec![0xAA, 0x55, 1, 0xFF]);
        assert_eq!(parse_hex("").unwrap(), Vec::<u8>::new());
        assert!(parse_hex("ABC").is_err());
        assert!(parse_hex("GG").is_err());
    }

    #[test]
    fn hex_format() {
        assert_eq!(format_hex(&[0x48, 0x65, 0x0A]), "48 65 0A");
    }

    #[test]
    fn ascii_format() {
        assert_eq!(format_ascii(b"hi\r\n"), "hi\\r\\n");
    }

    #[test]
    fn escapes() {
        assert_eq!(parse_escapes("\\r\\n").unwrap(), b"\r\n");
        assert_eq!(parse_escapes("\\x0D\\x0A\\0").unwrap(), vec![13, 10, 0]);
        assert!(parse_escapes("\\q").is_err());
        assert!(parse_escapes("\\x1").is_err());
    }
}
