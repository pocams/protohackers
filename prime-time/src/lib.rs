use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, info_span, warn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
struct Request {
    method: String,
    number: serde_json::Number,
}

#[derive(Debug, Clone, Serialize)]
struct Response {
    method: String,
    prime: bool,
}

struct ResponseLine {
    line: String,
    disconnect: bool,
}

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

fn get_response(request: &Request) -> Option<Response> {
    if request.method != "isPrime" {
        return None
    }

    let prime = if let Some(n) = request.number.as_u64() {
        if n == 0 || n == 1 {
            false
        } else {
            let sqrt = (n as f64).sqrt().floor() as u64;
            !(2..=sqrt).any(|x| n % x == 0)
        }
    } else {
        warn!(request=?request, "non-i64");
        false
    };

    Some(Response {
        method: "isPrime".to_string(),
        prime
    })
}

fn get_response_line(request_line: &str) -> ResponseLine {
    match serde_json::from_str::<Request>(request_line) {
        Ok(r) => {
            debug!(request=?r, "request");
            match get_response(&r) {
                None => {
                    warn!(request=?r, "bad request");
                    ResponseLine {
                        line: ":(".to_string(),
                        disconnect: true,
                    }
                }
                Some(r) => {
                    ResponseLine {
                        line: serde_json::to_string(&r).unwrap(),
                        disconnect: false
                    }
                }
            }
        }
        Err(e) => {
            error!(error=%e, "malformed request");
            ResponseLine {
                line: ":P".to_string(),
                disconnect: true
            }
        }
    }
}

async fn handle(stream: TcpStream, addr: SocketAddr) {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let mut connected = true;
    let span = info_span!("connection", client=%addr);
    while connected {
        match buf_reader.read_line(&mut line).await {
            Ok(n) => {
                debug!(client=%addr, bytes=n, line=line, "read ok");
                if n == 0 {
                    connected = false;
                } else {
                    let response_line = span.in_scope(|| get_response_line(&line));
                    let mut line = response_line.line;
                    line.push('\n');

                    match writer.write_all(line.as_bytes()).await {
                        Ok(()) => {
                            info!(client=%addr, line=line, "write ok");
                        }
                        Err(e) => {
                            info!(client=%addr, line=line, error=?e, "write failed");
                            connected = false;
                        }
                    }
                    if response_line.disconnect {
                        warn!(client=%addr, "disconnecting");
                        connected = false;
                    }
                }
                line.clear();
            }
            Err(e) => {
                warn!(client=%addr, error=%e, "read failed");
            }
        }
    }
    info!(client=%addr, "disconnect");
}
