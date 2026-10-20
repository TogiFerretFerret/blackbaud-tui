//! A one-shot local listener that receives the B2C callback URL from `capture.user.js`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::auth::parse_callback;
use crate::error::{Error, Result};

pub const PORT: u16 = 8420;

pub struct CaptureListener {
    listener: TcpListener,
}

impl CaptureListener {
    /// Bind before opening the browser so the userscript can never race us.
    pub fn bind(port: u16) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| Error::Auth(format!("cannot listen on 127.0.0.1:{port} ({e}); is another sign-in already waiting?")))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }

    pub fn port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Wait for a POST whose body is a B2C callback URL. Anything else that hits the port is ignored,
    /// so a stray local request cannot end the wait.
    pub fn wait(self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(body) = read_post_body(stream)
                        && parse_callback(&body).is_ok()
                    {
                        return Ok(body);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(e.into()),
            }
        }
        Err(Error::Auth(format!("no callback received within {}s", timeout.as_secs())))
    }
}

fn read_post_body(mut stream: TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 || buf.len() > 64 * 1024 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    while buf.len() < header_end + len.min(64 * 1024) {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    String::from_utf8(buf[header_end..].to_vec()).ok().map(|s| s.trim().to_owned())
}
