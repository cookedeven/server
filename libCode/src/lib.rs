use std::{
    error::Error,
    fmt::{Display, Formatter},
    sync::Arc,
    time::{Duration, SystemTime}
};
use tokio::{
    net::TcpStream,
    sync::Mutex
};
use uuid::Uuid;

pub const SERVER_IP: &'static str = "127.0.0.1:8080";
type ArcMutex<T> = Arc<Mutex<T>>;
type Name = String;

pub trait Log {}

pub struct MessageLog {
    data: Box<dyn Log + Send + Sync>
}

impl MessageLog {
    pub fn new(data: Box<dyn Log + Send + Sync>) -> Self {
        Self { data }
    }
}

pub struct ErrorLogging {
    error: Box<dyn Error + Send + Sync>,
    system_time: SystemTime,
    duration: Duration,
}

impl ErrorLogging {
    pub fn new(error: Box<dyn Error + Send + Sync>, system_time: SystemTime, duration: Duration) -> Self {
        Self { error, system_time, duration }
    }
}

impl Log for ErrorLogging {}

pub struct UpdateState {
    state: String,
    system_time: SystemTime,
    duration: Duration,
}

impl UpdateState {
    pub fn new(state: String, system_time: SystemTime, duration: Duration) -> Self {
        Self { state, system_time, duration }
    }
}

impl Log for UpdateState {}

#[derive(Debug)]
pub enum MessageError {
    NotFound,
    NotLongEnough,
    CommandNotFound,
    CommunicationError,
    UndefinedError,
    TooLong,
    OtherError(Box<dyn Error + Send + Sync>)
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

pub struct PlayerStream {
    pub tcp_stream: TcpStream,
    pub uuid: Uuid,
}
