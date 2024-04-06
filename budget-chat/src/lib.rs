use std::future::{pending};
use std::io;
use std::net::SocketAddr;
use futures::{FutureExt, StreamExt};
use futures::stream::FuturesUnordered;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tracing::{error, info, warn};

fn is_valid_nick(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Debug, Eq, PartialEq)]
enum ClientState {
    AwaitingNick,
    Connected,
    Disconnected
}

#[derive(Debug)]
struct ChatClient<C: AsyncRead + AsyncWrite> {
    reader: Lines<BufReader<ReadHalf<C>>>,
    writer: WriteHalf<C>,
    state: ClientState,
    nick: Option<String>,
}

impl<C: AsyncRead + AsyncWrite> ChatClient<C> {
    fn new(_addr: SocketAddr, stream: C) -> ChatClient<C> {
        let (r, w) = tokio::io::split(stream);
        let reader = BufReader::new(r).lines();
        ChatClient {
            reader,
            writer: w,
            state: ClientState::AwaitingNick,
            nick: None,
        }
    }

    async fn write_or_die(&mut self, message: &str) {
        match self.writer.write_all(message.as_bytes()).await {
            Ok(_) => {
            }
            Err(e) => {
                error!(error=?e, "write failed, closing");
                self.state = ClientState::Disconnected;
            }
        }
    }
}

async fn next_message<C: AsyncRead + AsyncWrite>(clients: &mut [ChatClient<C>]) -> (usize, Result<Option<String>, io::Error>) {
    let mut futures: FuturesUnordered<_> = clients
        .iter_mut()
        .enumerate()
        .map(|(i, c)| c.reader.next_line().map(move |line| (i, line)))
        .collect();
    match futures.next().await {
        None => { pending().await }
        Some((idx, msg)) => (idx, msg)
    }
}

pub async fn serve(address: SocketAddr) -> io::Result<()> {
    let mut clients: Vec<ChatClient<TcpStream>> = Vec::new();
    info!("starting");
    let listener = TcpListener::bind(address).await?;
    loop {
        clients.retain(|c| c.state != ClientState::Disconnected);

        let new_client = select! {
            incoming = listener.accept() => {
                match incoming {
                    Ok((stream, addr)) => {
                        info!(client=%addr, "connection received");
                        let mut client = ChatClient::new(addr, stream);
                        client.write_or_die("enter nick\n").await;
                        Some(client)
                    }

                    Err(e) => {
                        error!(error=?e, "accept failed");
                        None
                    }
                }
            }

            (client_idx, message) = next_message(&mut clients) => {
                match message {
                    Ok(Some(ref m)) => {
                        info!("client message: {:?} {:?}", clients[client_idx], m);
                        match clients[client_idx].state {
                            ClientState::AwaitingNick => {
                                let n = m.as_str().trim();
                                if is_valid_nick(n) {
                                    info!(nick=n, client=?clients[client_idx], "set nick");
                                    let in_room = format!("* in room: {}\n",
                                        clients.iter().filter_map(|i| i.nick.as_deref()).collect::<Vec<&str>>().join(", "));
                                    clients[client_idx].nick = Some(n.to_string());
                                    clients[client_idx].state = ClientState::Connected;
                                    clients[client_idx].write_or_die(in_room.as_str()).await;

                                    let entered = format!("* {} entered\n", n);
                                    for (i, c) in clients.iter_mut().enumerate() {
                                        if i != client_idx && c.state == ClientState::Connected {
                                            c.write_or_die(entered.as_str()).await;
                                        }
                                    }
                                } else {
                                    warn!(nick=n, client=?clients[client_idx], "invalid nick");
                                    clients[client_idx].write_or_die("invalid nick\n").await;
                                    clients[client_idx].state = ClientState::Disconnected;
                                }
                            }
                            ClientState::Connected => {
                                let said = format!("[{}] {}\n", clients[client_idx].nick.as_ref().expect("connected without nick"), m);
                                for (i, c) in clients.iter_mut().enumerate() {
                                    if i != client_idx && c.state == ClientState::Connected {
                                        c.write_or_die(said.as_str()).await;
                                    }
                                }
                            }
                            ClientState::Disconnected => unreachable!("we filtered out disconnected clients at the top of the loop")
                        }
                    }
                    Ok(None) | Err(_) => {
                        warn!(error=?message, "Client disconnect");
                        if clients[client_idx].state == ClientState::Connected {
                            let left = format!("* {} left\n", clients[client_idx].nick.as_ref().expect("connected without nick"));
                            for (i, c) in clients.iter_mut().enumerate() {
                                if i != client_idx {
                                    c.write_or_die(left.as_str()).await;
                                }
                            }
                        }
                        clients[client_idx].state = ClientState::Disconnected;
                    }
                }
                None
            }
        };

        if let Some(c) = new_client {
            clients.push(c);
        }
    }
}
