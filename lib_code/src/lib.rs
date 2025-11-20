use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
    str::{self, FromStr},
    sync::Arc,
    net::AddrParseError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, to_vec, Map, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::Mutex,
};
use dashmap::DashMap;

pub const SERVER_IP: &'static str = "127.0.0.1:8080";
pub type UserData = Map<String, Value>;
pub type AM<T> = Arc<Mutex<T>>;
pub type AD<K, V> = Arc<DashMap<K, V>>;

#[macro_export]
macro_rules! am {
    ($t:expr) => {
        Arc::new(Mutex::new($t))
    };
}

#[macro_export]
macro_rules! ad {
    () => {
        Arc::new(DashMap::new())
    };
}

#[macro_export]
macro_rules! send_data_setting {
    ($data:ident, $([$uuid:expr $(, ($key:expr, $value:expr))+]),+) => {
        $(
            let mut buffer = Map::new();

            $(
                buffer.insert($key, json!($value));
            )*
            $data.insert($uuid, json!(buffer));
        )*
    };
}

#[macro_export]
macro_rules! request_data_setting {
    ($request_data:ident, $([$uuid:expr $(, $value:expr)+]),+) => {
        $(
            let mut buffer = Vec::new();

            $(
                buffer.push(json!($value));
            )*
            $request_data.insert($uuid, json!(buffer));
        )*
    };
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TcpMessage {
    pub send_type: String,
    pub command: String,
    pub uuid: String,
    pub send_data: UserData, // Map<Uuid, Map<DataName, Value>>
    pub request_data: UserData // Map<Uuid, Vec<DataName>>
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Default)]
enum ErrorKind {
    #[default]
    None,
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
    InvalidUtf8,
    DeserializeError,
    SerializeError,
    OtherError,
    FatalError,
    Errors,
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Default)]
pub struct E {
    pub error_kind: ErrorKind,
    pub error: Option<Box<dyn StdError + Send + Sync>>,
}

impl Display for E {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "error kind: {}, error: {:?}", self.error_kind, self.error)
    }
}

#[derive(Debug, Default)]
pub enum Error {
    #[default]
    None,
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
    Errors(Vec<Self>),
    OtherError(Box<dyn StdError + Send + Sync>),
    FatalError(Box<dyn StdError + Send + Sync>)
}

impl Error {
    pub fn unrecoverable_error(&self) -> bool {
        match self {
            Error::NotFound => false,
            Error::NotLongEnough => false,
            Error::CommandNotFound => false,
            Error::CommunicationError => false,
            Error::UndefinedError => true,
            Error::TooLong => false,
            Error::EmptyCommand => false,
            Error::InvalidUUID => false,
            Error::MissingUUID => true,
            Error::ParseError => false,
            Error::ConnectionClosed => true,
            Error::InvalidType => false,
            Error::InvalidUtf8(_) => false,
            Error::DeserializeError(_) => false,
            Error::Errors(errors) => {
                errors.iter().any(|e| e.unrecoverable_error())
            }
            Error::OtherError(_) => false,
            Error::FatalError(_) => true,
            _ => false
        }
    }
}

impl StdError for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "message not found"),
            Self::NotLongEnough => write!(f, "message not long enough"),
            Self::CommandNotFound => write!(f, "command not found"),
            Self::CommunicationError => write!(f, "communication error"),
            Self::UndefinedError => write!(f, "undefined error"),
            Self::TooLong => write!(f, "message too long"),
            Self::EmptyCommand => write!(f, "empty command"),
            Self::InvalidUUID => write!(f, "invalid UUID format"),
            Self::MissingUUID => write!(f, "UUID is missing"),
            Self::ParseError => write!(f, "parse error"),
            Self::ConnectionClosed => write!(f, "connection closed"),
            Self::InvalidType => write!(f, "invalid type"),
            Self::InvalidUtf8(e) => write!(f, "invalid UTF8: {}", e),
            Self::DeserializeError(e) => write!(f, "deserialize error: {}", e),
            Self::Errors(errors) => {
                for error in errors {
                    write!(f, "{} ", error)?;
                }
                Ok(())
            }
            Self::OtherError(err) => write!(f, "{}", err),
            Self::FatalError(err) => write!(f, "fatal error: {}", err),
            _ => write!(f, "undefined unknown error"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::OtherError(Box::new(value))
    }
}

impl From<uuid::Error> for Error {
    fn from(value: uuid::Error) -> Self {
        Error::OtherError(value.into())
    }
}

impl From<AddrParseError> for Error {
    fn from(value: AddrParseError) -> Self {
        Error::OtherError(value.into())
    }
}

pub enum QueueType {
    Two,
    Four
}

#[derive(Default, Eq, PartialEq)]
pub enum MatchingState {
    #[default]
    None,
    Nothing,
    Matching,
    Wait,
    PlayWait,
    Playing
}

impl FromStr for MatchingState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "nothing" => Ok(Self::Nothing),
            "matching" => Ok(Self::Matching),
            "wait" => Ok(Self::Wait),
            "play_wait" => Ok(Self::PlayWait),
            "playing" => Ok(Self::Playing),
            _ => Err(Error::NotFound)
        }
    }
}

impl Display for MatchingState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}",
            match self {
                Self::None => "none",
                Self::Nothing => "nothing",
                Self::Matching => "matching",
                Self::Wait => "wait",
                Self::PlayWait => "play_wait",
                Self::Playing => "playing",
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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fatal" => Ok(Self::Fatal),
            "warning" => Ok(Self::Warning),
            "ok" => Ok(Self::Ok),
            _ => Err(Error::NotFound)
        }
    }
}

impl Display for ErrorLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}",
            match self {
                Self::Fatal => "fatal",
                Self::Warning => "warning",
                Self::Ok => "ok"
            }
        )
    }
}

pub async fn read_data(read_half: &mut OwnedReadHalf, buffer: &mut [u8]) -> Result<TcpMessage, Error> {
    let size= read_half.read(buffer).await.map_err(|e| Error::FatalError(e.into()))?;
    if size == 0 {
        return Err(Error::ConnectionClosed);
    }

    let message_bytes = &buffer[..size];
    let string_message = str::from_utf8(message_bytes).map_err(|e| Error::InvalidUtf8(e))?;

    println!("string_message: {}", string_message);

    let tcp_message: TcpMessage = serde_json::from_str(string_message).map_err(|e| Error::DeserializeError(e))?;

    println!("parse data: {:?}", tcp_message);

    Ok(tcp_message)
}

pub async fn send_tcp_message(write_half: &mut OwnedWriteHalf, tcp_message: TcpMessage) {
    let tcp_message_json = json!(tcp_message);

    match to_vec(&tcp_message_json) {
        Ok(tcp_message_json_byte) => {
            let _ = write_half.write_all(&tcp_message_json_byte).await;
        },
        Err(err) => eprintln!("err: {}", err)
    }
}
