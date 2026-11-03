pub struct Read<'a> {
    pub lines: Vec<&'a [u8]>,
    pub position: usize,
}

pub fn complete_lines(bytes: &[u8], position: usize) -> Read<'_> {
    let unread = bytes.get(position..).unwrap_or_default();
    let Some(last_newline) = unread.iter().rposition(|&byte| byte == b'\n') else {
        return Read {
            lines: Vec::new(),
            position,
        };
    };
    let lines = unread[..last_newline]
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    Read {
        lines,
        position: position + last_newline + 1,
    }
}
