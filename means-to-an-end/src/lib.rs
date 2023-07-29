use std::collections::BTreeMap;
use std::net::SocketAddr;
use bincode::Decode;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, info_span, warn};

#[derive(Decode, Debug)]
struct Request {
    command: u8,
    a: i32,
    b: i32,
}

#[derive(Debug, Default)]
struct ClientData {
    price_history: BTreeMap<i32, i32>
}

impl ClientData {
    fn apply_request(&mut self, request: &Request) -> Option<i32> {
        match request.command {
            b'I' => {
                let timestamp = request.a;
                let price = request.b;
                debug!(timestamp=timestamp, price=price, "insert");
                self.price_history.insert(timestamp, price);
                None
            }
            b'Q' => {
                let start = request.a;
                let end = request.b;
                debug!(start=start, end=end, "query");
                let mut total: i64 = 0;
                let mut count: i64 = 0;
                if end >= start {
                    for (_timestamp, price) in self.price_history.range(start..=end) {
                        total += *price as i64;
                        count += 1;
                    }
                }
                let average = total.checked_div(count).unwrap_or(0) as i32;
                debug!(total=total, count=count, average=average, "query result");
                Some(average)
            }
            _ => {
                error!(request=?request, "unexpected command");
                Some(-1)
            }
        }
    }
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

async fn handle(stream: TcpStream, addr: SocketAddr) {
    let bincode_config = bincode::config::standard()
        .with_big_endian()
        .with_fixed_int_encoding();

    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut command_buf = vec![0u8; 9];
    let mut data = ClientData::default();
    let mut connected = true;
    let span = info_span!("connection", client=%addr);

    while connected {
        match buf_reader.read_exact(&mut command_buf).await {
            Ok(n) => {
                debug!(client=%addr, bytes=n, data=?command_buf, "read ok");
                if n == 0 { connected = false; }
                let (request, _bytes_read): (Request, _) = bincode::decode_from_slice(&command_buf, bincode_config).unwrap();
                if let Some(reply) = span.in_scope(|| data.apply_request(&request)) {
                    let reply_buf = bincode::encode_to_vec(reply, bincode_config).unwrap();
                    debug!(client=%addr, data=?reply_buf, "sending reply");

                    match writer.write_all(&reply_buf).await {
                        Ok(()) => {
                            debug!(client=%addr, bytes=reply_buf.len(), "write ok");
                        }
                        Err(e) => {
                            warn!(client=%addr, error=%e, "write failed");
                            break;
                        }
                    }
                }
            }

            Err(e) => {
                warn!(client=%addr, error=%e, "read failed");
                connected = false
            }
        }
    }
    info!(client=%addr, "disconnect");
}
