use std::{
    fmt::{Display, Formatter},
    error::Error,
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, Instant, Duration}
};
use std::io::Read;
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;

type ShardState = Arc<Mutex<HashMap<String, UpdateState>>>;

const SERVER_IP: &'static str = "127.0.0.1:8080";

lazy_static! {
    static ref VEC_DB: Arc<Mutex<Vec<(Uuid, String)>>> = Arc::new(Mutex::new(Vec::new()));
}

lazy_static! {
    static ref ID_NAME: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
}

lazy_static! {
    static ref UID_UPDATE: ShardState = Arc::new(Mutex::new(HashMap::new()));
}

lazy_static! {
    static ref ProgramTime: Instant = Instant::now();
}

struct UpdateState {
    state: String,
    date: SystemTime,
    duration: Duration,
}

impl UpdateState {
    fn new(state: String, date: SystemTime, duration: Duration) -> Self {
        Self { state, date, duration }
    }
}

#[derive(Debug)]
enum MessageError {
    NotFound,
    NotLongEnough,
    CommandNotFound,
    CommunicationError,
    UndefinedError,
}

impl Error for MessageError {}

impl Display for MessageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "message not found"),
            Self::NotLongEnough => write!(f, "message not long enough"),
            Self::CommandNotFound => write!(f, "command not found"),
            Self::CommunicationError => write!(f, "communication error"),
            Self::UndefinedError => write!(f, "undefined error"),
            _ => write!(f, "unknown error"),
        }
    }
}

async fn tcp_new_uuid(message: &[u8], stream: &mut TcpStream) -> Result<(), MessageError> {
    match message.get(1) {
        Some(mode_2) => {
            match mode_2 {
                1 => {
                    let id = message[2] as usize;
                    if let Some((uuid, name)) = VEC_DB.lock().await.get(id) {
                        let uuid_name_chain: Vec<u8> = uuid.as_bytes().into_iter().cloned().chain(name.as_bytes().iter().cloned()).collect();
                        let _ = stream.write(&uuid_name_chain).await;
                        Ok(())
                    } else {
                        let _ = stream.write(b"invalid id").await;
                        Ok(())
                    }
                }
                2 => {
                    let new_name_len = message[2] as usize;
                    let Some(new_name_utf8) = message.get(3..new_name_len + 3) else {
                        return Err(MessageError::NotLongEnough)
                    };
                    let new_name = String::from_utf8_lossy(new_name_utf8).into_owned();
                    let uuid = Uuid::new_v4();
                    VEC_DB.lock().await.push((uuid, new_name));
                    let _ = stream.write(uuid.as_bytes()).await;
                    Ok(())
                }
                _ => Err(MessageError::CommandNotFound)
            }
        }
        None => Err(MessageError::NotLongEnough)
    }
}

async fn tcp_connect_uuid(message: &[u8], stream: &mut TcpStream, state: &ShardState) -> Result<(), MessageError> {
    let Some(uuid) = message.get(1..17) else {
        return Err(MessageError::NotLongEnough)
    };
    let mut state = state.lock().await;
    let key = String::from_utf8_lossy(uuid).into_owned();
    if state.contains_key(&key) {
        state.insert(String::from_utf8_lossy(uuid).into(), UpdateState::new("update!".into(), SystemTime::now(), ProgramTime.elapsed()));
        let _ = stream.write(format!("connect to UID! name: ").as_bytes()).await;
        Ok(())
    } else {
        let _ = stream.write(b"invalid UID").await;
        Ok(())
    }
}

async fn async_uuid_tcp_handle<>(mut stream: TcpStream, state: ShardState) -> Result<(), MessageError> {
    let mut buffer = vec![0; 1024];
    if let Ok(size) = stream.read(&mut buffer).await {
        let message = &buffer[..size];

        match message.first() {
            Some(mode_1) => {
                match mode_1 {
                    0 => Err(MessageError::NotFound),
                    1 => tcp_new_uuid(&message, &mut stream).await,
                    2 => tcp_connect_uuid(&message, &mut stream, &state).await,
                    _ => Err(MessageError::CommandNotFound)
                }
            }
            None => Err(MessageError::NotFound)
        }
    } else {
        Err(MessageError::CommunicationError)
    }
}

async fn uuid_tcp_server() {
    let listener = TcpListener::bind(SERVER_IP).await.unwrap();
    let state: ShardState = Arc::new(Mutex::new(HashMap::new()));
    println!("Server listening on {}", SERVER_IP);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                spawn(async move {
                    let _ = async_uuid_tcp_handle(stream, state).await;
                });
            }
            Err(err) => {
                eprintln!("Connection failed: {}", err);
            }
        }
    }
}

async fn uuid_udp_server() {
    let socket = UdpSocket::bind(SERVER_IP).await.unwrap();
    println!("UDP 서버! {}", SERVER_IP);

    let mut buf = vec![0; 1024];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, addr)) => {
                let msg = String::from_utf8_lossy(&buf[..n]);

                println!("UDP 메시지 form: {}: {}", addr, msg);

                let response = format!("서버 응답 (UDP): {}", msg);
                let _ = socket.send_to(response.as_bytes(), addr).await;
            }
            Err(e) => eprintln!("UDP 수신 오류: {}", e),
        }
    }
}

#[tokio::main]
async fn main() {
    tokio::join!(uuid_tcp_server());
}
