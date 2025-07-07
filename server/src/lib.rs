use std::{
    sync::Arc,
    collections::HashMap,
    str::{self, FromStr}
};
use std::fmt::{Display, format};
use tokio::{
    net::{TcpListener, TcpStream},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        Mutex,
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender}
    },
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, from_value, json};
use dashmap::DashMap;
use libCode::*;

pub type AM<T> = Arc<Mutex<T>>;
pub type Name = String;
pub type PlayerInfo = (Uuid, Name);
pub type PlayerQueue = (Uuid, Name);

#[macro_export]
macro_rules! am {
    ($t:expr) => {
        Arc::new(Mutex::new($t))
    };
}

macro_rules! return_tcp_stream_read_data {
    ($tcp_stream:expr, $buffer:expr, $tcp_message:ident) => {
        let size = match $tcp_stream.read($buffer).await {
            Ok(size) => size,
            Err(err) => return Err((MessageError::OtherError(err.into()), $tcp_stream))
        };

        let message = &$buffer[..size];

        let string_message = match String::from_utf8(message.to_vec()) {
            Ok(message) => message,
            Err(err) => return Err((MessageError::OtherError(err.into()), $tcp_stream))
        };
        
        println!("string_message: {}", string_message);

        let parse_data = match serde_json::from_str(&string_message) {
            Ok(value) => value,
            Err(err) => return Err((MessageError::OtherError(err.into()), $tcp_stream))
        };
        
        println!("parse data: {:?}", parse_data);

        let $tcp_message: TcpMessage = match from_value(parse_data) {
            Ok(tcp_message) => tcp_message,
            Err(err) => return Err((MessageError::OtherError(err.into()), $tcp_stream))
        };
    }
}

macro_rules! read_data {
    ($tcp_stream:expr, $buffer:expr, $tcp_message:ident) => {
        let size = match $tcp_stream.read($buffer).await {
            Ok(size) => size,
            Err(err) => return Err(MessageError::OtherError(err.into()))
        };

        let message = &$buffer[..size];

        let string_message = match String::from_utf8(message.to_vec()) {
            Ok(message) => message,
            Err(err) => return Err(MessageError::OtherError(err.into()))
        };

        let parse_data = json!(string_message);

        let $tcp_message: TcpMessage = match from_value(parse_data) {
            Ok(tcp_message) => tcp_message,
            Err(err) => return Err(MessageError::OtherError(err.into()))
        };
    }
}

lazy_static! {
    static ref PLAYER_TABLE: AM<HashMap<Uuid, AM<TcpStream>>> = am!(HashMap::new());
    static ref MATCH_QUEUE_2: AM<Vec<[PlayerInfo; 2]>> = am!(Vec::new());
    static ref MATCH_QUEUE_4: AM<Vec<[PlayerInfo; 4]>> = am!(Vec::new());
    static ref PLAYER_DATA: AM<Vec<PlayerInfo>> = am!(Vec::new());
    static ref PLAYER_QUEUE_2: AM<Vec<PlayerQueue>> = am!(Vec::new());
    static ref PLAYER_QUEUE_4: AM<Vec<PlayerQueue>> = am!(Vec::new());
}

#[derive(Serialize, Deserialize, Debug)]
struct TcpMessage {
    send_type: String,
    command: String,
    uuid: String,
    send_data: Map<String, Value>,
    request_data: Vec<String>
}

#[derive(Default)]
struct State {
    matching: bool,
    playing: bool
}

impl State {
    fn matching_change(&mut self, matching: bool) {
        self.matching = matching;
    }
    
    fn playing_change(&mut self, playing: bool) {
        self.playing = playing;
    }
}

fn search_id<T: PartialEq, U>(data: &[(T, U)], target: T) -> Option<usize> {
    data.iter().position(|(a, _)| *a == target)
}

fn search_uuid<T: PartialEq, U>(data: &[(T, U)], target: T) -> Option<&U> {
    data.iter().find(|(t, _)| *t == target).map(|(_, u)| u)
}

async fn tcp_match(uuid: Uuid, am_tcp_stream: AM<TcpStream>) -> Result<(), MessageError> {
    let player_list = PLAYER_DATA.lock().await;
    match search_uuid(&player_list, uuid) {
        Some(_) => {
            let mut player_queue_2 = PLAYER_QUEUE_2.lock().await;
            //PLAYER_QUEUE_2.push()
        }
        None => return Err(MessageError::NotFound)
    }

    Ok(())
}

/*
async fn match_handle(message: &TcpMessage, am_tcp_stream: AM<TcpStream>, uuid: Uuid, name: &Name, wait: &mut bool, playing: &mut bool) -> Result<(), MessageError> {
    let mut PLAYER_QUEUE_2 = PLAYER_QUEUE_2.lock().await;
    let mut tcp_stream = am_tcp_stream.lock().await;

    match message.command.as_str() {



        "JOIN" => {
            let queue_type = match message.send_data.get(MATCH_QUEUE_TYPE_POS) {
                Some(&"1vs1") => Ok(QueueType::Two),
                Some(&"2vs2") => Ok(QueueType::Four),
                Some(_) => Err(MessageError::CommandNotFound),
                None => Err(MessageError::NotLongEnough)
            }?;
            PLAYER_QUEUE_2.push((uuid, name.clone(), queue_type));
            let _ = tcp_stream.write_all(format!("MATCH.JOIN.{}", uuid).as_bytes());
            *wait = true;
        }
        "LEAVE" => {
            PLAYER_QUEUE_2.retain(|(queue_uuid, _queue_name, _queue_type)| *queue_uuid != uuid);
            let _ = tcp_stream.write_all(format!("MATCH.LEAVE.{}", uuid).as_bytes());
            *wait = false;
        }
        _ => return Err(MessageError::CommandNotFound),
    }

    Ok(())
}
 */

async fn connect_handle(message: &TcpMessage, am_tcp_stream: AM<TcpStream>) -> Result<(), MessageError> {
    /*
    match message.command.as_str() {
        "send" => {

        }
    }
    */
    todo!()
}

async fn uuid_check(mut tcp_stream: TcpStream, buffer: &mut [u8]) -> Result<(AM<TcpStream>, Uuid), (MessageError, TcpStream)> {
    return_tcp_stream_read_data!(tcp_stream, buffer, tcp_message);

    if &tcp_message.send_type == &"uuid" {
        match tcp_message.command.as_str() {
            "get" => {
                let uuid = Uuid::new_v4();
                let mut player_table = PLAYER_TABLE.lock().await;
                let am_tcp_stream = am!(tcp_stream);
                player_table.insert(uuid, Arc::clone(&am_tcp_stream));
                Ok((am_tcp_stream, uuid))
            }
            "check" => {
                let uuid = match Uuid::from_str(&tcp_message.uuid) {
                    Ok(uuid) => uuid,
                    Err(_) => return Err((MessageError::InvalidUUID, tcp_stream)),
                };
                let player_table = PLAYER_TABLE.lock().await;
                match player_table.get(&uuid) {
                    Some(stream) => Ok((Arc::clone(stream), uuid)),
                    None => Err((MessageError::NotFound, tcp_stream))
                }
            }
            _ => Err((MessageError::CommandNotFound, tcp_stream))
        }
    } else {
        Err((MessageError::NotFound, tcp_stream))
    }
}

async fn name_check(am_tcp_stream: AM<TcpStream>, buffer: &mut [u8]) -> Result<Name, MessageError> {
    let mut tcp_stream = am_tcp_stream.lock().await;
    read_data!(tcp_stream, buffer, tcp_message);

    if tcp_message.send_type == "name" {
        let key = "name".to_string();
        let Some(value) = tcp_message.send_data.get(&key) else {
            return Err(MessageError::NotFound)
        };
        match value {
            Value::String(name) => Ok(name.into()),
            _ => Err(MessageError::NotFound)
        }
    } else {
        Err(MessageError::CommandNotFound)
    }
}

fn send_data_handle(am_tcp_stream: AM<TcpStream>, send_data: Map<String, Value>, uuid: &Uuid, name: &Name, state: &mut State) {
    if send_data.is_empty() {
        return;
    }
    
    match send_data {
        Map { .. } => {}
    }
}

fn request_data_handle(am_tcp_stream: AM<TcpStream>, request_data: Vec<String>, uuid: &Uuid, name: &Name, state: &mut State) {
    if request_data.is_empty() {
        return;
    }
}

async fn message_handle(am_tcp_stream: AM<TcpStream>, uuid: Uuid, name: &Name, state: &mut State, buffer: &mut [u8]) -> Result<(), MessageError> {
    let mut tcp_stream = am_tcp_stream.lock().await;

    read_data!(tcp_stream, buffer, tcp_message);

    if tcp_message.send_type.as_str() == "connect" {
        send_data_handle(Arc::clone(&am_tcp_stream), tcp_message.send_data, &uuid, name, state);
        request_data_handle(Arc::clone(&am_tcp_stream), tcp_message.request_data, &uuid, name, state);
    }

    Err(MessageError::NotFound)
}

async fn async_uuid_tcp_handle(am_tcp_stream: AM<TcpStream>, uuid: Uuid, main_write: ClientThreadMessageSender, thread_read: ClientMessageReceiver) {
    let mut buffer = vec![0; 1024];
    println!("new client! uuid: {}", uuid);
    
    let name = match name_check(Arc::clone(&am_tcp_stream), &mut buffer).await {
        Ok(name) => name,
        Err(err) => {
            eprintln!("err: {}", err);
            let mut tcp_stream = am_tcp_stream.lock().await;
            let _ = tcp_stream.write_all(format!("err: {}\n Connect End", err).as_bytes()).await;
            return;
        }
    };
    
    let mut state = State::default();
    
    loop {
        tokio::select! {
            _ = async {
                message_handle(Arc::clone(&am_tcp_stream), uuid, &name, &mut state, &mut buffer)
            } => {}
        }
    }
}

pub async fn uuid_tcp_server(server_read: ClientMessageReceiver, server_write: ServerMessageSender) {
    let Ok(listener) = TcpListener::bind(SERVER_IP).await else {
        panic!("cannot bind {}", SERVER_IP)
    };
    println!("Server listening on {}", SERVER_IP);

    let (main_thread_write, mut main_thread_read) = unbounded_channel();
    let thread_write_map = Arc::new(DashMap::new());

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                println!("new client!!!!!!! addr: {}", addr);
                let server_message = ServerMessage::new(Uuid::max(), format!("new client! addr: {}", addr));
                let _ = server_write.send(server_message);
                let mut buffer = vec![0; 512];
                let (am_tcp_stream, uuid) = match uuid_check(stream, &mut buffer).await {
                    Ok((am_tcp_stream, uuid)) => {
                        let mut tcp_stream = am_tcp_stream.lock().await;
                        let _ = tcp_stream.write_all(format!("set to uuid!\n your uuid is: {}", uuid).as_bytes()).await;
                        (Arc::clone(&am_tcp_stream), uuid)
                    },
                    Err((err, mut tcp_stream)) => {
                        eprintln!("err: {}", err);
                        let _ = tcp_stream.write_all(format!("err: {}\n Connect End", err).as_bytes()).await;
                        return;
                    }
                };

                let (thread_write, thread_read) = unbounded_channel();
                
                thread_write_map.insert(uuid, thread_write);

                let main_thread_write = main_thread_write.clone();

                spawn(async move {
                    async_uuid_tcp_handle(am_tcp_stream, uuid, main_thread_write, thread_read).await
                });
            }
            Err(err) => eprintln!("tcp server err: {}", err)
        }
    }
}

pub async fn server_loop(mut clients_read: ServerMessageReceiver, client_write: ClientMessageSender) {
    // let (main_thread_write, mut main_thread_read) = unbounded_channel();
    // let thread_write_map = Arc::new(DashMap::new());
    loop {
        tokio::task::yield_now().await;
    }
}
