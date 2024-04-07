use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use nom::branch::alt;
use nom::bytes::streaming::tag;
use nom::IResult;
use nom::combinator::map;
use nom::multi::length_count;
use nom::number::streaming::{be_u16, be_u32, be_u8};
use nom::sequence::tuple;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::time::interval;
use tracing::{debug, error, info};

#[derive(Debug, Eq, PartialEq)]
struct Camera {
    road: u16,
    mile: u16,
    limit: u16,
}

#[derive(Debug, Eq, PartialEq)]
struct Dispatcher {
    roads: Vec<u16>,
}

#[derive(Debug, Eq, PartialEq)]
enum ClientType {
    Unknown,
    Camera(Camera),
    Dispatcher(Dispatcher),
}

trait ToMsg {
    fn to_msg(&self) -> Vec<u8>;
}

#[derive(Debug)]
struct ErrorMsg {
    message: String
}

impl ErrorMsg {
    fn msg(s: &str) -> ErrorMsg {
        ErrorMsg { message: s.to_string() }
    }
}

#[derive(Debug)]
struct Ticket {
    plate: Vec<u8>,
    road: u16,
    mile1: u16,
    timestamp1: u32,
    mile2: u16,
    timestamp2: u32,
    speed: u16, // (100x miles per hour)
}

#[derive(Debug)]
struct WantHeartbeat {
    interval: u32
}

struct Heartbeat {}

impl ToMsg for Heartbeat {
    fn to_msg(&self) -> Vec<u8> {
        vec![0x41u8]
    }
}

impl ToMsg for &[u8] {
    fn to_msg(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.len() + 1);
        v.push(self.len() as u8);
        v.extend_from_slice(self);
        v
    }
}

impl ToMsg for ErrorMsg {
    fn to_msg(&self) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.push(b'\x10');
        msg.extend_from_slice(&self.message.as_bytes().to_msg());
        msg
    }
}

impl ToMsg for Ticket {
    fn to_msg(&self) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.push(b'\x21');
        msg.extend_from_slice(&self.plate.as_slice().to_msg());
        msg.extend_from_slice(&self.road.to_be_bytes());
        msg.extend_from_slice(&self.mile1.to_be_bytes());
        msg.extend_from_slice(&self.timestamp1.to_be_bytes());
        msg.extend_from_slice(&self.mile2.to_be_bytes());
        msg.extend_from_slice(&self.timestamp2.to_be_bytes());
        msg.extend_from_slice(&self.speed.to_be_bytes());
        msg
    }
}

struct PlateReport {
    plate: Vec<u8>,
    timestamp: u32
}

impl Debug for PlateReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlateReport {{ plate: {}, timestamp: {} }}", String::from_utf8_lossy(&self.plate), self.timestamp)
    }
}

#[derive(Debug)]
struct IAmCamera {
    road: u16,
    mile: u16,
    limit: u16,
}

#[derive(Debug)]
struct IAmDispatcher {
    roads: Vec<u16>
}

#[derive(Debug, Eq, PartialEq)]
struct Observation {
    road: u16,
    mile: u16,
    timestamp: u32,
}

impl Ord for Observation {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

impl PartialOrd for Observation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Default)]
struct Database {
    speed_limits: HashMap<u16, u16>,
    observations: HashMap<Vec<u8>, Vec<Observation>>,
    tickets_issued: HashMap<Vec<u8>, Vec<u32>>,
    tickets_to_send: Vec<Ticket>,
}

impl Database {
    fn record_speed_limit(&mut self, road: u16, limit: u16) {
        self.speed_limits.insert(road, limit);
    }

    fn record_observation(&mut self, plate: Vec<u8>, road: u16, mile: u16, timestamp: u32) {
        let obs = self.observations.entry(plate.clone()).or_default();
        obs.push(Observation { road, mile, timestamp });
        obs.sort_unstable();
        if obs.len() > 1 {
            self.issue_tickets(&plate, road);
        }
    }

    fn issue_tickets(&mut self, plate: &[u8], road: u16) {
        let &limit = self.speed_limits.get(&road).expect("no speed limit for observed road");
        let obs: Vec<_> = self.observations.get(plate).map(|os| os.iter().filter(|o| o.road == road).collect()).unwrap_or_default();
        let p = String::from_utf8_lossy(plate);

        for w in obs.windows(2) {
            let [o1, o2] = w else { panic!("non-2-sized windows: {w:?}") };
            let speed = o2.mile.abs_diff(o1.mile) as f64 / ((o2.timestamp - o1.timestamp) as f64 / 3600.0);
            // debug!(plate=%p, road=road, o1=?o1, o2=?o2, speed=speed, limit=limit, "observed");
            if speed > (limit as f64) + 0.1 {
                // Issue a ticket
                let day1 = o1.timestamp / 86400;
                let day2 = o2.timestamp / 86400;
                let issued = self.tickets_issued.entry(plate.to_owned()).or_default();
                if issued.contains(&day1) || (day1 != day2 && issued.contains(&day2)) {
                    debug!(plate=%p, day1=day1, day2=day2, "already issued ticket on this day");
                } else {
                    if !issued.contains(&day1) { issued.push(day1) };
                    if day1 != day2 && !issued.contains(&day2) { issued.push(day2) };
                    let ticket = Ticket {
                        plate: plate.to_owned(),
                        road,
                        mile1: o1.mile,
                        timestamp1: o1.timestamp,
                        mile2: o2.mile,
                        timestamp2: o2.timestamp,
                        speed: (speed * 100.0).round() as u16,
                    };
                    info!(plate=%p, ticket=?ticket, "issuing ticket");
                    self.tickets_to_send.push(ticket);
                }
            }
        }
    }

    fn get_ticket_to_send(&mut self, roads: &[u16]) -> Option<Ticket> {
        self.tickets_to_send.iter()
            .position(|t| roads.contains(&t.road))
            .map(|p| self.tickets_to_send.remove(p))
    }
}

fn parse_str(input: &[u8]) -> IResult<&[u8], Vec<u8>> {
    length_count(
        be_u8,
        be_u8
    )(input)
}

fn parse_plate(input: &[u8]) -> IResult<&[u8], PlateReport> {
    tuple((
        tag(b"\x20"),
        parse_str,
        be_u32
    ))(input)
        .map(|(rest, (_, plate, timestamp))| (rest, PlateReport { plate, timestamp }))
}

fn parse_wantheartbeat(input: &[u8]) -> IResult<&[u8], WantHeartbeat> {
    tuple((
        tag(b"\x40"),
        be_u32
    ))(input)
        .map(|(rest, (_, interval))| (rest, WantHeartbeat { interval }))
}

fn parse_iamcamera(input: &[u8]) -> IResult<&[u8], IAmCamera> {
    tuple((
        tag(b"\x80"),
        be_u16,
        be_u16,
        be_u16
    ))(input)
        .map(|(rest, (_, road, mile, limit))| (rest, IAmCamera { road, mile, limit }))
}

fn parse_iamdispatcher(input: &[u8]) -> IResult<&[u8], IAmDispatcher> {
    tuple((
        tag(b"\x81"),
        length_count(be_u8, be_u16)
    ))(input)
        .map(|(rest, (_, roads))| (rest, IAmDispatcher { roads }))
}

#[derive(Debug)]
enum IncomingPacket {
    WantHeartbeat(WantHeartbeat),
    IAmCamera(IAmCamera),
    IAmDispatcher(IAmDispatcher),
    PlateReport(PlateReport),
}

fn parse_incoming(input: &[u8]) -> IResult<&[u8], IncomingPacket> {
    alt((
        map(parse_plate, |r| IncomingPacket::PlateReport(r)),
        map(parse_wantheartbeat, |r| IncomingPacket::WantHeartbeat(r)),
        map(parse_iamcamera, |r| IncomingPacket::IAmCamera(r)),
        map(parse_iamdispatcher, |r| IncomingPacket::IAmDispatcher(r)),
    ))(input)
}

pub async fn serve(address: SocketAddr) -> io::Result<()> {
    info!("starting");
    let listener = TcpListener::bind(address).await?;

    let database = Arc::new(Mutex::new(Database::default()));

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!(client=%addr, "connection received");
                tokio::spawn(handle(stream, addr, database.clone()));
            }
            Err(e) => {
                error!(error=?e, "accept failed");
            }
        }
    }
}

async fn handle(mut stream: TcpStream, addr: SocketAddr, database: Arc<Mutex<Database>>) {
    let mut heartbeat = interval(Duration::from_secs(86400*365));
    // First tick happens right away
    heartbeat.tick().await;
    let mut requested_heartbeat = false;

    let mut dispatch_interval = interval(Duration::from_secs(86400*365));
    // First tick happens right away
    dispatch_interval.tick().await;

    let mut client_type = ClientType::Unknown;
    let mut buf = Vec::with_capacity(1024);
    loop {
        select! {
            _ = heartbeat.tick() => {
                if requested_heartbeat {
                    debug!(addr=?addr, "sending heartbeat");
                    if let Err(e) = stream.write_all(&(Heartbeat {}.to_msg())).await {
                        error!(addr=?addr, err=?e, "Heartbeat failed");
                        return;
                    }
                }
            }

            _ = dispatch_interval.tick() => {
                match client_type {
                    ClientType::Dispatcher(ref d) => {
                        debug!(addr=?addr, dispatcher=?d, "checking for tickets");
                        while let Some(t) = {
                            let mut db = database.lock().unwrap();
                            let t = db.get_ticket_to_send(&d.roads);
                            t
                        } {
                            info!(addr=?addr, ticket=?t, "dispatching ticket");
                            if let Err(e) = stream.write_all(&t.to_msg()).await {
                                error!(addr=?addr, err=?e, "write ticket failed");
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }

            b = stream.read_buf(&mut buf) => {
                match b {
                    Ok(n) if n > 0 => { }
                    _ => {
                        error!(addr=?addr, recv=?b, "lost connection");
                        return;
                    }
                }
                // debug!(addr=?addr, bytes=?b, "bytes received");
                loop {
                    match parse_incoming(&buf) {
                        Ok((left, packet)) => {
                            buf = left.to_vec();
                            info!(addr=?addr, packet=?packet, client=?client_type, "packet received");
                            match packet {
                                IncomingPacket::WantHeartbeat(h) => {
                                    if requested_heartbeat {
                                        error!(addr=?addr, "already requested heartbeat");
                                        let _ = stream.write_all(&ErrorMsg::msg("already requested heartbeat").to_msg()).await;
                                        return;
                                    }
                                    info!(addr=?addr, interval=h.interval, "want heartbeat");
                                    requested_heartbeat = true;
                                    if h.interval != 0 {
                                        heartbeat = interval(Duration::from_millis((h.interval * 100) as u64));
                                        heartbeat.tick().await;
                                    }
                                }
                                IncomingPacket::IAmCamera(c) => {
                                    if client_type != ClientType::Unknown {
                                        error!(addr=?addr, "already sent client type");
                                        let _ = stream.write_all(&ErrorMsg::msg("already sent client type").to_msg()).await;
                                        return;
                                    }
                                    database.lock().unwrap().record_speed_limit(c.road, c.limit);
                                    client_type = ClientType::Camera(
                                        Camera { road: c.road, limit: c.limit, mile: c.mile }
                                    );
                                }
                                IncomingPacket::IAmDispatcher(d) => {
                                    if client_type != ClientType::Unknown {
                                        error!(addr=?addr, "already sent client type");
                                        let _ = stream.write_all(&ErrorMsg::msg("already sent client type").to_msg()).await;
                                        return;
                                    }
                                    client_type = ClientType::Dispatcher(
                                        Dispatcher { roads: d.roads }
                                    );
                                    dispatch_interval = interval(Duration::from_secs(1));
                                }
                                IncomingPacket::PlateReport(p) => {
                                    if let ClientType::Camera(ref c) = client_type {
                                        database.lock().unwrap().record_observation(p.plate, c.road, c.mile, p.timestamp);
                                    } else {
                                        error!(addr=?addr, client_type=?client_type, "unexpected PlateReport");
                                        let _ = stream.write_all(&ErrorMsg::msg("wrong client type").to_msg()).await;
                                        return;
                                    }
                                }
                            }
                        }
                        Err(nom::Err::Incomplete(_)) => break,
                        Err(e) => {
                            error!(addr=?addr, error=?e, "invalid input");
                            let _ = stream.write_all(&ErrorMsg::msg("invalid input").to_msg()).await;
                            return;
                        }
                    }
                }
            }
        }
    }
}
