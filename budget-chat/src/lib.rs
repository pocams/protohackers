use std::net::SocketAddr;
use std::ptr::write;
use std::sync::atomic::{AtomicU64, Ordering};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::{io, select};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::net::{TcpListener, TcpStream};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info, warn};

static CLIENT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct ClientMessage {
    from_client_id: u64,
    message: MessageType,
}

#[derive(Debug, Clone)]
enum MessageType {
    Joined(String),
    Message(String, String),
    Left(String),
}

#[derive(Debug)]
struct Client<W: AsyncWrite, R: AsyncRead> {
    client_id: u64,
    nick: Option<String>,
    to_network: BufWriter<W>,
    // from_network: BufReader<R>,
    from_network: Lines<BufReader<R>>,
    to_server: Sender<ClientMessage>,
    from_server: Receiver<ClientMessage>,
}

#[derive(Debug)]
struct ClientHandle {
    to_client: Sender<ClientMessage>,
    from_client: Receiver<ClientMessage>,
}

#[derive(Debug)]
struct Server {
    clients: Vec<ClientHandle>,
    new_clients: Receiver<ClientHandle>,
}

#[derive(Debug)]
struct ServerHandle {
    new_clients: Sender<ClientHandle>
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Client<W, R> {
    fn start(network: TcpStream) -> ClientHandle {
        let (read_half, write_half) = network.into_split();
        let (to_server, from_client) = mpsc::channel(1024);
        let (to_client, from_server) = mpsc::channel(1024);
        let client = Client {
            client_id: CLIENT_ID.fetch_add(1, Ordering::SeqCst),
            nick: None,
            to_network: BufWriter::new(write_half),
            from_network: BufReader::new(read_half).lines(),
            to_server,
            from_server
        };
        tokio::spawn(client.run());
        ClientHandle {
            to_client,
            from_client
        }
    }

    fn set_nick(&mut self, nick: String) -> Result<(), ()> {
        if self.nick.is_some() {
            panic!("can't set_nick() twice");
        }
        if !nick.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(());
        }
        self.nick = Some(nick);
        Ok(())
    }

    fn message(&self, message: MessageType) -> ClientMessage {
        ClientMessage {
            from_client_id: self.client_id,
            message,
        }
    }

    async fn run(mut self) {
        info!("client running");
        let mut connected = true;
        let _ = self.to_network.write_all(b"hi what your nick\n").await;
        while connected {
            select! {
                line = self.from_network.next_line() => {
                    if let Ok(Some(l)) = line {
                        if let Some(n) = self.nick.as_ref() {
                            let _ = self.to_server.send(self.message(MessageType::Message(n.clone(), l))).await;
                        } else {
                            if self.set_nick(l).is_ok() {
                                let _ = self.to_server.send(self.message(MessageType::Joined(self.nick.as_ref().unwrap().clone()))).await;
                            } else {
                                let _ = self.to_network.write_all(b":x\n").await;
                                connected = false;
                            }
                        }
                    }
                }

                message = self.from_server.recv() => {
                    if let Some(message) = message {
                        if message.from_client_id != self.client_id {
                            if self.nick.is_some() {
                                let msg = match message.message {
                                    MessageType::Joined(nick) => format!("* {} has joined", nick),
                                    MessageType::Message(nick, msg) => format!("[{}] {}", nick, msg),
                                    MessageType::Left(nick) => format!("* {} has left", nick),
                                };

                                let mut m = msg.as_bytes().to_vec();
                                m.push(ba'\n');
                                if let Err(e) = self.to_network.write_all(&m).await {
                                    error!(error=?e, "write failed");
                                    connected = false;
                                }
                            }
                        }
                    } else {
                        error!("server went away");
                        connected = false;
                    }
                }
            }
        }

        if let Some(n) = self.nick.as_ref() {
            let _ = self.to_server.send(self.message(MessageType::Left(n.clone()))).await;
        }
    }
}

impl Server {
    fn start() -> ServerHandle {
        let (clients_to_server, clients_from_listener) = mpsc::channel(1024);
        let server = Server {
            clients: vec![],
            new_clients: clients_from_listener
        };
        tokio::spawn(server.run());
        ServerHandle {
            new_clients: clients_to_server
        }
    }

    async fn run(mut self) {
        info!("server running");
        loop {
            let mut client_receives: FuturesUnordered<_> = self.clients.iter_mut().map(|c| c.from_client.recv()).collect();

            select! {
                new_client = self.new_clients.recv() => {
                    drop(client_receives);
                    if let Some(new_client) = new_client {
                        debug!("server: added new client {:?}", new_client);
                        self.clients.push(new_client);
                    } else {
                        error!("handler went away");
                        break;
                    }
                }

                Some(Some(message)) = client_receives.next() => {
                    drop(client_receives);
                    debug!("server: client message {:?}", message);
                    // Ignore the possibility that clients will disconnect for now
                    for client in self.clients.iter_mut() {
                        let _ = client.to_client.send(message.clone()).await;
                    }
                }
            }
        }
    }
}

pub async fn serve(listener: TcpListener) {
    info!("starting");
    let server_handle = Server::start();
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!(client=%addr, "connection received");
                let client_handle = Client::<OwnedWriteHalf, OwnedReadHalf>::start(stream);
                server_handle.new_clients.send(client_handle).await.unwrap();
            }

            Err(e) => {
                error!(error=?e, "accept failed");
            }
        }
    }
}
