use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

pub async fn serve(listener: TcpListener) {
    info!("starting");
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

async fn handle(mut stream: TcpStream, addr: SocketAddr) {
    let mut buf = Vec::with_capacity(1024);
    let mut connected = true;
    while connected {
        match stream.read_buf(&mut buf).await {
            Ok(n) => {
                debug!(client=%addr, bytes=n, data=%String::from_utf8_lossy(&buf), "read ok");
                if n == 0 { connected = false; }

                match stream.write_all(&buf).await {
                    Ok(()) => {
                        debug!(client=%addr, bytes=buf.len(), "write ok");
                        buf.clear();

                    }
                    Err(e) => {
                        warn!(client=%addr, error=%e, "write failed");
                        break;
                    }
                }
            }
            Err(e) => {
                warn!(client=%addr, error=%e, "read failed");
            }
        }
    }
    info!(client=%addr, "disconnect");
}
