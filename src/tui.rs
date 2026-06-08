//! Minimal interactive terminal helpers — no TUI dependency.
//!
//! [`select`] presents a list the user navigates with the arrow keys (Enter to
//! choose), with number keys still working as a shortcut. It uses raw terminal
//! mode via `libc` (canonical mode + echo off) and restores the terminal on any
//! exit through a [`RawMode`] drop guard. When raw mode can't be entered it
//! falls back to a numbered prompt, so it degrades instead of failing. The
//! caller is responsible for only invoking this on an interactive TTY.

use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;

/// A decoded keypress relevant to list navigation.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Enter,
    /// A 1-based number shortcut.
    Digit(usize),
    /// Ctrl-C, `q`, Esc, or EOF — abandon the selection.
    Cancel,
    Other,
}

/// Decode a key from a raw byte sequence (`ESC [ A/B` for arrows, CR/LF for
/// Enter, `1`–`9` for shortcuts, Ctrl-C/`q` to cancel). Pure, so it's unit
/// tested without a terminal.
fn decode_key(seq: &[u8]) -> Key {
    match seq {
        [b'\r', ..] | [b'\n', ..] => Key::Enter,
        [3, ..] | [b'q', ..] | [b'Q', ..] => Key::Cancel,
        [27, b'[', b'A', ..] => Key::Up,
        [27, b'[', b'B', ..] => Key::Down,
        // A lone Esc (or any other Esc-sequence) abandons, matching `q`/Ctrl-C.
        [27, ..] => Key::Cancel,
        [d @ b'1'..=b'9', ..] => Key::Digit((d - b'0') as usize),
        _ => Key::Other,
    }
}

/// Present `items` as an arrow-navigable list; returns the chosen index, or
/// `None` if the user cancelled (Ctrl-C / `q` / Esc / EOF). Numbers select
/// directly. Falls back to a numbered prompt when raw mode is unavailable.
pub fn select(items: &[String]) -> io::Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    match RawMode::enable() {
        Some(_guard) => arrow_select(items),
        None => number_select(items),
    }
}

fn arrow_select(items: &[String]) -> io::Result<Option<usize>> {
    let mut idx = 0usize;
    paint(items, idx, false)?;
    let mut stdin = io::stdin();
    loop {
        match read_key(&mut stdin)? {
            Key::Up => {
                idx = (idx + items.len() - 1) % items.len();
                paint(items, idx, true)?;
            }
            Key::Down => {
                idx = (idx + 1) % items.len();
                paint(items, idx, true)?;
            }
            Key::Enter => return Ok(Some(idx)),
            Key::Digit(n) if (1..=items.len()).contains(&n) => return Ok(Some(n - 1)),
            Key::Cancel => return Ok(None),
            _ => {}
        }
    }
}

/// Draw the list. `repaint` first moves the cursor back up over the previously
/// drawn rows so the list updates in place.
fn paint(items: &[String], idx: usize, repaint: bool) -> io::Result<()> {
    let mut out = io::stdout();
    if repaint {
        // Move up over the rows we drew last time, to the first row.
        write!(out, "\x1b[{}A", items.len())?;
    }
    let color = colors_enabled();
    for (i, item) in items.iter().enumerate() {
        // Clear the row, then draw it. Explicit CR keeps columns sane in raw mode.
        if i == idx {
            let (p, b, r) = if color {
                ("\x1b[36m❯\x1b[0m", "\x1b[1m", "\x1b[0m")
            } else {
                ("\u{276f}", "", "")
            };
            write!(out, "\x1b[2K {p} {b}{item}{r}\r\n")?;
        } else {
            write!(out, "\x1b[2K   {item}\r\n")?;
        }
    }
    out.flush()
}

fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// Read one keypress. Reads a byte; if it's Esc, opportunistically pulls the
/// rest of the escape sequence so arrow keys decode in one shot.
fn read_key(stdin: &mut io::Stdin) -> io::Result<Key> {
    let mut buf = [0u8; 3];
    if stdin.read(&mut buf[..1])? == 0 {
        return Ok(Key::Cancel); // EOF
    }
    if buf[0] == 27 {
        // Esc — try to read the two trailing bytes of an arrow sequence.
        let _ = stdin.read(&mut buf[1..2])?;
        let _ = stdin.read(&mut buf[2..3])?;
    }
    Ok(decode_key(&buf))
}

/// Numbered fallback used when raw mode can't be entered (returns the chosen
/// 0-based index, or `None` on EOF).
fn number_select(items: &[String]) -> io::Result<Option<usize>> {
    let mut out = io::stdout();
    for (i, item) in items.iter().enumerate() {
        writeln!(out, "   {}) {}", i + 1, item)?;
    }
    loop {
        write!(out, " ❯ ")?;
        out.flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Ok(n) = line.trim().parse::<usize>() {
            if (1..=items.len()).contains(&n) {
                return Ok(Some(n - 1));
            }
        }
        writeln!(out, "  enter a number between 1 and {}.", items.len())?;
    }
}

/// RAII guard: puts the terminal in raw mode (input only — output post-processing
/// is left on, so `\n` still works) and restores the original settings on drop,
/// including on an early return or a panic-unwind.
struct RawMode {
    fd: i32,
    orig: libc::termios,
}

impl RawMode {
    fn enable() -> Option<RawMode> {
        let fd = io::stdin().as_raw_fd();
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut orig) != 0 {
                return None;
            }
            let mut raw = orig;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(RawMode { fd, orig })
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.orig);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_navigation_keys() {
        assert_eq!(decode_key(&[27, b'[', b'A']), Key::Up);
        assert_eq!(decode_key(&[27, b'[', b'B']), Key::Down);
        assert_eq!(decode_key(&[b'\r', 0, 0]), Key::Enter);
        assert_eq!(decode_key(&[b'\n', 0, 0]), Key::Enter);
        assert_eq!(decode_key(&[3, 0, 0]), Key::Cancel); // Ctrl-C
        assert_eq!(decode_key(&[b'q', 0, 0]), Key::Cancel);
        assert_eq!(decode_key(&[27, 0, 0]), Key::Cancel); // lone Esc
        assert_eq!(decode_key(&[b'2', 0, 0]), Key::Digit(2));
        assert_eq!(decode_key(&[b'x', 0, 0]), Key::Other);
    }
}
