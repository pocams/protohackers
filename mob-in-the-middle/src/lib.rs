use std::io;
use std::net::SocketAddr;
use std::sync::OnceLock;
use tokio::net::TcpListener;
use regex::bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::select;
use tracing::{debug, error, info};

const SERVER: (&str, u16) = ("chat.protohackers.com", 16963);


pub async fn serve(address: SocketAddr) -> io::Result<()> {
    let listener = TcpListener::bind(address).await?;
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!(client=%addr, "connection received");
                tokio::spawn(handle(stream, addr));
            }
            Err(e) => {
                error!(error=?e, "accept failed");
            }
        }
    }
}

fn transform_line(line: &[u8]) -> Vec<u8> {
    // No lookahead/lookbehind available, so we use this technique to look for spaces before/after
    // https://docs.rs/regex/latest/regex/struct.Regex.html#fallibility
    static RE: OnceLock<bytes::Regex> = OnceLock::new();
    let mut replaced = Vec::new();
    let mut last_match = 0;
    for cap in RE.get_or_init(|| bytes::Regex::new(r"\b7\w{25,34}\b").unwrap())
        .captures_iter(line) {
        let m = cap.get(0).unwrap();
        replaced.extend_from_slice(&line[last_match..m.start()]);
        if (m.start() == 0 || line[m.start() - 1] == b' ') &&
            (m.end() == line.len() || line[m.end()] == b' ') {
            replaced.extend_from_slice(b"7YWHMfk9JZe0LM0g1ZauHuiSxhI");
        } else {
            // Not a real boguscoin address, leave as-is
            replaced.extend_from_slice(m.as_bytes());
        }
        last_match = m.end();
    }
    replaced.extend_from_slice(&line[last_match..]);
    replaced.push(b'\n');
    replaced
}

fn next_line(b: &mut Vec<u8>) -> Option<Vec<u8>> {
    if let Some(newline) = b.iter().position(|&c| c == b'\n') {
        let line = b[..newline].to_vec();
        b.drain(..newline+1);
        Some(line)
    } else {
        None
    }
}

async fn handle(mut client: TcpStream, addr: SocketAddr) -> io::Result<()> {
    let mut server = TcpStream::connect(SERVER).await?;
    debug!(client=?addr, server=?server, "established server connection");
    let (mut from_client, mut to_client) = client.split();
    let (mut from_server, mut to_server) = server.split();
    let mut from_client_buf = Vec::with_capacity(1024);
    let mut from_server_buf = Vec::with_capacity(1024);
    loop {
        select! {
            b = from_client.read_buf(&mut from_client_buf) => {
                match b {
                    Ok(count) if count > 0 => {
                        while let Some(line) = next_line(&mut from_client_buf) {
                            debug!(client=?addr, line=%String::from_utf8_lossy(&line), "from client");
                            to_server.write_all(&transform_line(&line)).await?;
                        }
                    }
                    _ => {
                        error!(client=?addr, error=?b, "lost client connection");
                        return Ok(())
                    }
                }
            }
            b = from_server.read_buf(&mut from_server_buf) => {
                match b {
                    Ok(count) if count > 0 => {
                        while let Some(line) = next_line(&mut from_server_buf) {
                            debug!(client=?addr, line=%String::from_utf8_lossy(&line), "from server");
                            to_client.write_all(&transform_line(&line)).await?;
                        }
                    }
                    _ => {
                        error!(client=?addr, error=?b, "lost client connection");
                        return Ok(())
                    }
                }
            }
        }
    }
}
