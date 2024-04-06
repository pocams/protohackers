use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tracing::{debug, error, warn};

struct Database {
    data: HashMap<Vec<u8>, Vec<u8>>
}

impl Database {
    fn new() -> Database {
        Database { data: HashMap::new() }
    }

    fn set(&mut self, key: Vec<u8>, value: Vec<u8>) {
        debug!(key=%String::from_utf8_lossy(&key), value=%String::from_utf8_lossy(&value), "set");
        if key == b"version" {
            warn!("ignoring set of 'version'");
            return;
        }
        self.data.insert(key, value);
    }

    fn get<'s>(&'s self, key: &[u8]) -> Option<&'s [u8]> {
        debug!(key=%String::from_utf8_lossy(key), "get");
        if key == b"version" {
            return Some(b"Unusual Database Program");
        }
        return self.data.get(key).map(|s| s.as_slice())
    }
}


pub async fn serve(address: SocketAddr) -> io::Result<()> {
    let sock = UdpSocket::bind(address).await?;
    let mut buf = vec![0u8; 1024];
    let mut database = Database::new();
    loop {
        buf.resize(1024, 0);
        match sock.recv_from(&mut buf).await {
            Ok((bytes, src)) => {
                if bytes > 1000 {
                    error!(bytes=bytes, "too many bytes received");
                    continue;
                }
                buf.truncate(bytes);
                debug!(message=%String::from_utf8_lossy(&buf), src=?src, "message");

                if let Some(equals) = buf.iter().position(|&c| c == b'=') {
                    let key = buf[..equals].to_vec();
                    let value = buf[equals+1..].to_vec();
                    database.set(key, value);
                } else if let Some(value) = database.get(&buf) {
                    let mut response = buf.clone();
                    response.push(b'=');
                    response.extend_from_slice(value);
                    match sock.send_to(&response, src).await {
                        Ok(b) => { debug!(length=b, response=%String::from_utf8_lossy(&response), "sent reply") }
                        Err(e) => { error!(error=?e, "failed to send") }
                    }
                }
            }
            Err(e) => {
                error!(err=?e, "receiving packet")
            }
        }
    }
}
