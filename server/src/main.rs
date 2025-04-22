use std::{
    fmt::{Display, Formatter},
    error::Error,
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, Instant, Duration},
    ops::Range,
};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;

type ShardState = Arc<Mutex<HashMap<String, MessageLog>>>;

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
    static ref PROGRAM_TIME: Instant = Instant::now();
}

lazy_static! {
    static ref LOG: Arc<Mutex<Vec<MessageLog>>> =  Arc::new(Mutex::new(Vec::new()));
}

/*
클라이언트 -> 서버 메시지 규격
index 0: command(get, new, connect)
index 1-4: 게임 세션 ID
index 5-17: UUID
index 27-: 정보
*/

/*
서버 -> 클라이언트 메시지 규격
index 0-16: UUID
index 17-25: 게임 세션 ID
index 26-: 정보
*/

/*
정보 규격
index 0: 데이터 1의 크기
index 1-index 0: 데이터 1
index index 0 + 1: 데이터 2의 크기
index index 0 + 2-index index 0 + 1" 데이터 2
...
*/

const COMMAND_RANGE: Range<usize> = 0..1;
const SESSION_ID_RANGE: Range<usize> = 0..4;
const UUID_RANGE: Range<usize> = 0..16;

const CLIENT_TO_SERVER_COMMAND_RANGE: Range<usize> = range_offset(COMMAND_RANGE, 0);
const CLIENT_TO_SERVER_SESSION_ID_RANGE: Range<usize> = range_offset(SESSION_ID_RANGE, 1);
const CLIENT_TO_SERVER_GET_UUID_RANGE: Range<usize> = range_offset(UUID_RANGE, 1);
const CLIENT_TO_SERVER_CONNECT_UUID_RANGE: Range<usize> = range_offset(UUID_RANGE, 4);
const SERVER_TO_CLIENT_SESSION_ID_RANGE: Range<usize> = range_offset(SESSION_ID_RANGE, 0);
const SERVER_TO_CLIENT_UUID_RANGE: Range<usize> = range_offset(UUID_RANGE, 4);

const fn range_offset(range: Range<usize>, offset: usize) -> Range<usize> {
    (range.start + offset)..(range.end + offset)
}

trait Log {}

struct MessageLog {
    data: Box<dyn Log + Send + Sync>
}

impl MessageLog {
    fn new(data: Box<dyn Log + Send + Sync>) -> Self {
        Self { data }
    }
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

impl Log for UpdateState {}

#[derive(Debug)]
enum MessageError {
    NotFound,
    NotLongEnough,
    CommandNotFound,
    CommunicationError,
    UndefinedError,
    TooLong,
    OtherError(Box<dyn Error>)
}

impl Error for MessageError {}

impl Display for MessageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "message not found"),
            Self::NotLongEnough => write!(f, "message not long enough"),
            Self::CommandNotFound => write!(f, "command not found"),
            Self::CommunicationError => write!(f, "communication error"),
            Self::TooLong => write!(f, "too long"),
            Self::UndefinedError => write!(f, "undefined error"),
            Self::OtherError(err) => write!(f, "{}", err),
            _ => write!(f, "unknown error"),
        }
    }
}

fn search_id<T: PartialEq, U>(db: &[(T, U)], target: T) -> Option<usize> {
    db.iter().position(|(a, b)| *a == target)
}

async fn tcp_get_uuid(message: &[u8], stream: &mut TcpStream) -> Result<(), MessageError> {
    let bytes_id = match message.get(CLIENT_TO_SERVER_GET_UUID_RANGE) {
        Some(byte_id) => byte_id,
        None => return Err(MessageError::NotLongEnough)
    };
    let byte_id: [u8; 8] = match bytes_id.try_into() {
        Ok(byte_id) => byte_id,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };
    let u64_id = u64::from_le_bytes(byte_id);
    let id: usize = match u64_id.try_into() {
        Ok(id) => id,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };
    if let Some((uuid, name)) = VEC_DB.lock().await.get(id) {
        let uuid_name_chain: Vec<u8> = uuid.as_bytes().into_iter().cloned().chain(name.as_bytes().iter().cloned()).collect();
        let _ = stream.write_all(&uuid_name_chain).await;
        Ok(())
    } else {
        let _ = stream.write_all(b"invalid id").await;
        Ok(())
    }
}

async fn tcp_new_uuid(message: &[u8], stream: &mut TcpStream) -> Result<(), MessageError> {
    let new_name_len = message[1] as usize;
    let Some(new_name_utf8) = message.get(2..new_name_len + 2) else {
        return Err(MessageError::NotLongEnough)
    };
    let new_name = String::from_utf8_lossy(new_name_utf8).into_owned();
    let uuid = Uuid::new_v4();
    VEC_DB.lock().await.push((uuid, new_name));
    let _ = stream.write_all(uuid.as_bytes()).await;
    if message.len() > new_name_len + 2 {
        Err(MessageError::TooLong)
    } else {
        Ok(())
    }
}


async fn tcp_connect_uuid(message: &[u8], stream: &mut TcpStream, state: &ShardState) -> Result<(), MessageError> {
    let Some(session_id) = message.get(CLIENT_TO_SERVER_SESSION_ID_RANGE) else {
        return Err(MessageError::NotLongEnough)
    };
    let Some(uuid) = message.get(CLIENT_TO_SERVER_CONNECT_UUID_RANGE) else {
        return Err(MessageError::NotLongEnough)
    };
    let mut state = state.lock().await;
    let key = String::from_utf8_lossy(uuid).into_owned();
    if state.contains_key(&key) {
        state.insert(String::from_utf8_lossy(uuid).into(), MessageLog::new(Box::new(UpdateState::new("update!".into(), SystemTime::now(), PROGRAM_TIME.elapsed()))));
        let _ = stream.write_all(format!("connect to UID! uid: {}", key).as_bytes()).await;
        Ok(())
    } else {
        let _ = stream.write_all(b"invalid UID").await;
        Ok(())
    }
}

async fn async_uuid_tcp_handle(mut stream: TcpStream, state: ShardState) -> Result<(), MessageError> {
    let mut buffer = vec![0; 1024];
    loop {
        match stream.read(&mut buffer).await {
            Ok(size) => {
                let message = &buffer[..size];

                let result = match message.first() {
                    Some(mode_1) => {
                        match mode_1 {
                            0 => Err(MessageError::NotFound),
                            1 => return Ok(()),
                            2 => tcp_get_uuid(&message, &mut stream).await,
                            3 => tcp_new_uuid(&message, &mut stream).await,
                            4 => tcp_connect_uuid(&message, &mut stream, &state).await,
                            _ => Err(MessageError::CommandNotFound)
                        }
                    }
                    None => return Err(MessageError::NotFound)
                };

                if let Err(err) = result {
                    return Err(err)
                }
            }
            Err(err) => return Err(MessageError::OtherError(err.into()))
        }
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
                    match async_uuid_tcp_handle(stream, state).await {
                        Ok(()) => (),
                        Err(err) => eprintln!("err: {}", err)
                    };
                });
            }
            Err(err) => eprintln!("tcp server err: {}", err)
        }
    }
}

async fn uuid_udp_parse_input(uuid: [u8; 16], other_message: &[u8]) -> Result<(), MessageError> {
    let uuid = Uuid::from_bytes(uuid);
    let Some(id) = search_id(&VEC_DB.lock().await, uuid) else {
        return Err(MessageError::NotFound)
    };

    let (uuid, name) = match VEC_DB.lock().await.get(id) {
        Some(container) => container,
        None => return Err(MessageError::NotFound)
    };



    todo!()
}

async fn uuid_udp_server() -> Result<(), MessageError> {
    let socket = UdpSocket::bind(SERVER_IP).await.unwrap();

    let mut buf = vec![0; 1024];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((size, _addr)) => {
                let message = &buf[..size];
                let session_id: [u8; 4] = match message.get(SERVER_TO_CLIENT_SESSION_ID_RANGE) {
                    Some(session_id) => match session_id.try_into() {
                        Ok(session_id) => session_id,
                        Err(err) => return Err(MessageError::OtherError(err.into()))
                    }
                    None => return Err(MessageError::NotLongEnough)
                };
                let uuid: [u8; 16] = match message.get(SERVER_TO_CLIENT_UUID_RANGE) {
                    Some(uuid) => match uuid.try_into() {
                        Ok(uuid) => uuid,
                        Err(err) => return Err(MessageError::OtherError(err.into()))
                    },
                    None => return Err(MessageError::NotLongEnough)
                };
                let other_message = match message.get(16..) {
                    Some(other_message) => other_message,
                    None => return Err(MessageError::NotLongEnough)
                };
                match uuid_udp_parse_input(uuid, other_message).await {
                    Ok(_) => (),
                    Err(err) => return Err(err)
                }
            }
            Err(err) => return Err(MessageError::OtherError(err.into())),
        }
    }
}

#[tokio::main]
async fn main() {
    tokio::join!(uuid_tcp_server());
}
