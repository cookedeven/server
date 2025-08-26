use std::{
    error::Error,
    fmt::{Display, Formatter},
    sync::Arc,
    time::{Duration, SystemTime},
    str::FromStr
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, mpsc::{UnboundedReceiver, UnboundedSender}},
};
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const SERVER_IP: &'static str = "127.0.0.1:8080";
type AM<T> = Arc<Mutex<T>>;
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
    EmptyCommand,
    InvalidUUID,
    MissingUUID,
    ParseError,
    ConnectionClosed,
    InvalidType,
    InvalidUtf8(std::str::Utf8Error),
    DeserializeError(serde_json::Error),
    OtherError(Box<dyn Error + Send + Sync>),
    FatalError(Box<dyn Error + Send + Sync>)
}

impl Error for MessageError {}

impl Display for MessageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "message not found"),
            Self::NotLongEnough => write!(f, "message not long enough"),
            Self::CommandNotFound => write!(f, "command not found"),
            Self::CommunicationError => write!(f, "communication error"),
            Self::TooLong => write!(f, "message too long"),
            Self::UndefinedError => write!(f, "undefined error"),
            Self::EmptyCommand => write!(f, "empty command"),
            Self::InvalidUUID => write!(f, "invalid UUID format"),
            Self::MissingUUID => write!(f, "UUID is missing"),
            Self::OtherError(err) => write!(f, "{}", err),
            Self::FatalError(err) => write!(f, "fatal error: {}", err),
            _ => write!(f, "undefined unknown error"),
        }
    }
}

impl From<std::io::Error> for MessageError {
    fn from(value: std::io::Error) -> Self {
        MessageError::OtherError(Box::new(value))
    }
}

impl From<uuid::Error> for MessageError {
    fn from(value: uuid::Error) -> Self {
        MessageError::OtherError(value.into())
    }
}

pub enum QueueType {
    Two,
    Four
}

pub struct PlayerStream {
    pub tcp_stream: TcpStream,
    pub uuid: Uuid,
}

pub type ClientMessageSender = UnboundedSender<ClientMessage>;
pub type ClientMessageReceiver = UnboundedReceiver<ClientMessage>;
pub type ServerMessageSender = UnboundedSender<ServerMessage>;
pub type ServerMessageReceiver = UnboundedReceiver<ServerMessage>;
pub type ClientThreadMessageSender = UnboundedSender<ClientThreadMessage>;
pub type ClientThreadMessageReceiver = UnboundedReceiver<ClientThreadMessage>;
pub type ServerThreadMessageSender = UnboundedSender<ServerThreadMessage>;
pub type ServerThreadMessageReceiver = UnboundedReceiver<ServerThreadMessage>;

pub struct ClientMessage {
    uuid: Uuid,
    context: String
}

impl ClientMessage {
    pub fn new(uuid: Uuid, context: String) -> Self {
        Self { uuid, context }
    }
}

pub struct ServerMessage {
    uuid: Uuid,
    context: String
}

impl ServerMessage {
    pub fn new(uuid: Uuid, context: String) -> Self {
        Self { uuid, context }
    }
}

pub struct ClientThreadMessage {
    uuid: Uuid,
    context: String
}

impl ClientThreadMessage {
    pub fn new(uuid: Uuid, context: String) -> Self {
        Self { uuid, context }
    }
}

pub struct ServerThreadMessage {
    uuid: Uuid,
    context: String
}

impl ServerThreadMessage {
    pub fn new(uuid: Uuid, context: String) -> Self {
        Self { uuid, context }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TcpMessage {
    pub send_type: String,
    pub command: String,
    pub uuid: String,
    pub send_data: UserData,
    pub request_data: UserData
}

pub type UserData = Map<String, Value>; // Map<Uuid, Map<DataName, Value>>

#[derive(Default, Eq, PartialEq)]
pub enum MatchingState {
    #[default]
    Nothing,
    Matching,
    Wait,
    PlayWait,
    Playing
}

impl FromStr for MatchingState {
    type Err = MessageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "nothing" => Ok(MatchingState::Nothing),
            "matching" => Ok(MatchingState::Matching),
            "wait" => Ok(MatchingState::Wait),
            "play_wait" => Ok(MatchingState::PlayWait),
            "playing" => Ok(MatchingState::Playing),
            _ => Err(MessageError::NotFound)
        }
    }
}

impl Display for MatchingState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}",
            match self {
                MatchingState::Nothing => "nothing",
                MatchingState::Matching => "matching",
                MatchingState::Wait => "wait",
                MatchingState::PlayWait => "play_wait",
                MatchingState::Playing => "playing",
            }
        )
    }
}

pub enum ErrorLevel {
    Fatal,
    Warning,
    Ok
}

impl FromStr for ErrorLevel {
    type Err = MessageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fatal" => Ok(ErrorLevel::Fatal),
            "warning" => Ok(ErrorLevel::Warning),
            "ok" => Ok(ErrorLevel::Ok),
            _ => Err(MessageError::NotFound)
        }
    }
}

impl Display for ErrorLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}",
            match self {
                ErrorLevel::Fatal => "fatal",
                ErrorLevel::Warning => "warning",
                ErrorLevel::Ok => "ok"
            }
        )
    }
}
