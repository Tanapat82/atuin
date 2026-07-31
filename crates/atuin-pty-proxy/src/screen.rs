use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::compositor::Compositor;

pub(crate) fn socket_path() -> PathBuf {
    let dir = std::env::temp_dir();
    dir.join(format!("atuin-pty-proxy-{}.sock", std::process::id()))
}

/// Serve screen snapshots to local clients (e.g. `atuin search`, which uses
/// them to restore the screen area its popup covered).
pub(crate) fn spawn_socket_server<W: Write + Send + 'static>(
    sock_path: PathBuf,
    compositor: Arc<Mutex<Compositor<W>>>,
) {
    std::thread::spawn(move || {
        let listener = match UnixListener::bind(&sock_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("atuin pty-proxy: failed to bind socket: {e}");
                return;
            }
        };

        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };

            let data = match compositor.lock() {
                Ok(compositor) => encode_screen(compositor.screen()),
                Err(_) => break,
            };
            let _ = stream.write_all(&data);
            let _ = stream.flush();
        }
    });
}

/// Wire format written to the Unix socket:
///
/// ```text
/// [rows: u16 BE][cols: u16 BE][cursor_row: u16 BE][cursor_col: u16 BE]
/// [row_0_len: u32 BE][row_0_bytes...]
/// [row_1_len: u32 BE][row_1_bytes...]
/// ...
/// ```
///
/// Each row's bytes come from `screen.rows_formatted(0, cols)` and contain
/// pre-built ANSI escape sequences. The client can write them directly to
/// stdout without needing its own vt100 parser.
fn encode_screen(screen: &vt100::Screen) -> Vec<u8> {
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();

    let mut buf: Vec<u8> = Vec::with_capacity(256 + (rows as usize * cols as usize));
    buf.extend_from_slice(&rows.to_be_bytes());
    buf.extend_from_slice(&cols.to_be_bytes());
    buf.extend_from_slice(&cursor_row.to_be_bytes());
    buf.extend_from_slice(&cursor_col.to_be_bytes());

    for row_bytes in screen.rows_formatted(0, cols) {
        let len = row_bytes.len() as u32;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&row_bytes);
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::encode_screen;

    #[test]
    fn encode_screen_wire_format_is_stable() {
        let mut parser = vt100::Parser::new(3, 20, 0);
        parser.process(b"hello");
        let data = encode_screen(parser.screen());

        assert_eq!(u16::from_be_bytes([data[0], data[1]]), 3);
        assert_eq!(u16::from_be_bytes([data[2], data[3]]), 20);
        assert_eq!(u16::from_be_bytes([data[4], data[5]]), 0);
        assert_eq!(u16::from_be_bytes([data[6], data[7]]), 5);

        // Three length-prefixed rows follow.
        let mut offset = 8;
        let mut rows = 0;
        while offset + 4 <= data.len() {
            let len = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4 + len;
            rows += 1;
        }
        assert_eq!(rows, 3);
        assert_eq!(offset, data.len());
    }
}
