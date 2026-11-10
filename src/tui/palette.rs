use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::style::SECONDARY;

const MUTED_SHARE: f64 = 0.30;
const REPLY_WAIT: Duration = Duration::from_millis(150);
const LATE_REPLY_WAIT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    muted: Color,
}

impl Palette {
    pub fn between(text: Rgb, background: Rgb) -> Palette {
        let Rgb(red, green, blue) = blend(background, text, MUTED_SHARE);
        Palette {
            muted: Color::Rgb(red, green, blue),
        }
    }
}

pub fn blend(from: Rgb, to: Rgb, share: f64) -> Rgb {
    let channel = |from: u8, to: u8| {
        (from as f64 + (to as f64 - from as f64) * share)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgb(
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

pub fn apply(buffer: &mut Buffer, palette: Option<&Palette>) {
    for cell in buffer.content.iter_mut() {
        if cell.fg == SECONDARY {
            cell.fg = palette.map_or(Color::DarkGray, |palette| palette.muted);
        }
    }
}

pub fn parse_replies(replies: &[u8]) -> (Option<Rgb>, Option<Rgb>) {
    let text = String::from_utf8_lossy(replies);
    (
        color_reply(&text, "\x1b]10;"),
        color_reply(&text, "\x1b]11;"),
    )
}

fn color_reply(text: &str, prefix: &str) -> Option<Rgb> {
    let rest = &text[text.find(prefix)? + prefix.len()..];
    let end = rest.find(['\x1b', '\x07'])?;
    let mut channels = rest[..end].strip_prefix("rgb:")?.split('/').map(channel);
    let rgb = Rgb(channels.next()??, channels.next()??, channels.next()??);
    channels.next().is_none().then_some(rgb)
}

fn channel(hex: &str) -> Option<u8> {
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let max = (1u32 << (4 * hex.len())) - 1;
    Some((value * 255 / max) as u8)
}

pub fn query() -> Option<(Rgb, Rgb)> {
    let mut tty = File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    ratatui::crossterm::terminal::enable_raw_mode().ok()?;
    let replies = ask(&mut tty);
    let _ = ratatui::crossterm::terminal::disable_raw_mode();
    match parse_replies(&replies?) {
        (Some(text), Some(background)) => Some((text, background)),
        _ => None,
    }
}

fn ask(tty: &mut File) -> Option<Vec<u8>> {
    tty.write_all(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[c")
        .ok()?;
    tty.flush().ok()?;
    let mut replies = Vec::new();
    if read_until_answered(tty, REPLY_WAIT, &mut replies) {
        return Some(replies);
    }
    read_until_answered(tty, LATE_REPLY_WAIT, &mut replies);
    // SAFETY: tcflush only takes an open descriptor and a queue selector.
    unsafe { libc::tcflush(tty.as_raw_fd(), libc::TCIFLUSH) };
    None
}

fn read_until_answered(tty: &mut File, wait: Duration, replies: &mut Vec<u8>) -> bool {
    let deadline = Instant::now() + wait;
    while !answered_device_attributes(replies) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        if !readable_within(tty, left) {
            return false;
        }
        let mut chunk = [0u8; 256];
        let Ok(read) = tty.read(&mut chunk) else {
            return false;
        };
        replies.extend_from_slice(&chunk[..read]);
    }
    true
}

// macOS poll() reports POLLNVAL for /dev/tty, so select() is the only wait that works there.
#[cfg(target_os = "macos")]
fn readable_within(tty: &File, wait: Duration) -> bool {
    let fd = tty.as_raw_fd();
    let mut timeout = libc::timeval {
        tv_sec: wait.as_secs() as libc::time_t,
        tv_usec: wait.subsec_micros() as libc::suseconds_t,
    };
    // SAFETY: the set is zeroed, then holds one open descriptor below FD_SETSIZE, and nfds is fd + 1.
    unsafe {
        let mut readable: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&mut readable);
        libc::FD_SET(fd, &mut readable);
        let selected = libc::select(
            fd + 1,
            &mut readable,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut timeout,
        );
        selected > 0 && libc::FD_ISSET(fd, &readable)
    }
}

#[cfg(not(target_os = "macos"))]
fn readable_within(tty: &File, wait: Duration) -> bool {
    let mut ready = libc::pollfd {
        fd: tty.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `ready` is one valid pollfd for an open descriptor, matching the count of 1.
    let polled = unsafe { libc::poll(&mut ready, 1, wait.as_millis() as libc::c_int) };
    polled > 0 && ready.revents & libc::POLLIN != 0
}

fn answered_device_attributes(replies: &[u8]) -> bool {
    let text = String::from_utf8_lossy(replies);
    text.find("\x1b[?")
        .is_some_and(|start| text[start..].contains('c'))
}
