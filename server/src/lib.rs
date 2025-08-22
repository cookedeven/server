use std::{
    sync::Arc,
    collections::HashMap,
    str::{self, FromStr},
    fs::File
};
use tokio::{
    net::{
        TcpListener,
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf}
    },
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        Mutex,
        RwLock,
        mpsc::unbounded_channel
    },
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, json, to_vec};
use dashmap::DashMap;
use libCode::*;

pub type AM<T> = Arc<Mutex<T>>;
pub type ARL<T> = Arc<RwLock<T>>;
pub type Name = String;
pub type PlayerInfo = (Uuid, Name);
pub type PlayerQueue = (Uuid, Name);
pub type PlayerData = ARL<HashMap<Uuid, ARL<HashMap<String, Value>>>>;
pub type SessionID = i64;

#[macro_export]
macro_rules! am {
    ($t:expr) => {
        Arc::new(Mutex::new($t))
    };
}

#[macro_export]
macro_rules! arl {
    ($t:expr) => {
        Arc::new(RwLock::new($t))
    };
}

lazy_static! {
    static ref PLAYER_TABLE: AM<HashMap<Uuid, (AM<OwnedReadHalf>, AM<OwnedWriteHalf>)>> = am!(HashMap::new());
    static ref DATA_FILE: AM<File> = am!(File::open("./data.txt").expect("Failed to open file"));
    static ref USER_DATA: PlayerData = arl!(HashMap::new());
    static ref USER_SESSION_DATA: ARL<HashMap<SessionID, PlayerData>> = arl!(HashMap::new());
}

#[derive(Serialize, Deserialize, Debug)]
struct TcpMessage {
    send_type: String,
    command: String,
    uuid: String,
    send_data: Map<String, Value>,
    request_data: Map<String, Value>
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

async fn read_data(read_half: &mut OwnedReadHalf, buffer: &mut [u8]) -> Result<TcpMessage, MessageError> {
    let size = read_half.read(buffer).await.map_err(|e| MessageError::OtherError(e.into()))?;
    if size == 0 {
        return Err(MessageError::ConnectionClosed);
    }
    
    let size = match read_half.read(buffer).await {
        Ok(size) => size,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };

    let message_bytes = &buffer[..size];
    let string_message = str::from_utf8(message_bytes).map_err(|e| MessageError::InvalidUtf8(e))?;

    println!("string_message: {}", string_message);

    let tcp_message = serde_json::from_str::<TcpMessage>(string_message).map_err(|e| MessageError::DeserializeError(e))?;

    println!("parse data: {:?}", tcp_message);

    Ok(tcp_message)
}

async fn send_tcp_message(write_half: &mut OwnedWriteHalf, tcp_message: TcpMessage) {
    let tcp_message_json = json!(tcp_message);
        
    match to_vec(&tcp_message_json) {
        Ok(tcp_message_json_byte) => {
            let _ = write_half.write_all(&tcp_message_json_byte).await;
        },
        Err(err) => eprintln!("err: {}", err)
    }
}

async fn uuid_check(tcp_stream: TcpStream, buffer: &mut [u8]) -> Result<((AM<OwnedReadHalf>, AM<OwnedWriteHalf>), Uuid), (MessageError, OwnedReadHalf, OwnedWriteHalf)> {
    let (mut read_half, write_half) = tcp_stream.into_split();
    
    let tcp_message = match read_data(&mut read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err((err, read_half, write_half))
    };

    if &tcp_message.send_type == "uuid" {
        match tcp_message.command.as_str() {
            "get" => {
                let uuid = Uuid::new_v4();
                let mut player_table = PLAYER_TABLE.lock().await;
                let am_read_write_half = (am!(read_half), am!(write_half));
                player_table.insert(uuid, am_read_write_half.clone());
                Ok((am_read_write_half, uuid))
            }
            "check" => {
                let uuid = match Uuid::from_str(&tcp_message.uuid) {
                    Ok(uuid) => uuid,
                    Err(_) => return Err((MessageError::InvalidUUID, read_half, write_half)),
                };
                let player_table = PLAYER_TABLE.lock().await;
                match player_table.get(&uuid) {
                    Some(stream) => Ok((stream.clone(), uuid)),
                    None => Err((MessageError::NotFound, read_half, write_half))
                }
            }
            _ => Err((MessageError::CommandNotFound, read_half, write_half))
        }
    } else {
        Err((MessageError::NotFound, read_half, write_half))
    }
}

async fn name_check(read_half: &mut OwnedReadHalf, buffer: &mut [u8]) -> Result<Name, MessageError> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

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

fn send_data_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, send_data: Map<String, Value>, uuid: &Uuid, name: &Name, state: &mut State) {
    if send_data.is_empty() {
        return;
    }
    
    let data: Vec<_> = send_data.iter().map(|(k, v)| {
        match k.as_str() {
            "a" => "a",
            _ => "b"
        }
    }).collect();
    
}

fn request_data_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, request_data: Map<String, Value>, uuid: &Uuid, name: &Name, state: &mut State) {
    if request_data.is_empty() {
        return;
    }
    
    let data: Vec<_> = request_data.iter().map(|(k, v)| { 
        let target_= match Uuid::from_str(k) {
            Ok(uuid) => uuid,
            Err(_) => return Err(MessageError::InvalidUUID)
        };
        
        match v {
            Value::String(request) => {
                match request.as_str() {
                    "a" => Ok("1"),
                    _ => Ok("2")
                }
            },
            _ => Err(MessageError::InvalidType)
        }
    }).collect();
}

async fn message_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, uuid: Uuid, name: &Name, state: &mut State, buffer: &mut [u8]) -> Result<(), MessageError> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

    if tcp_message.send_type.as_str() == "connect" {
        send_data_handle(read_half, write_half, tcp_message.send_data, &uuid, name, state);
        request_data_handle(read_half, write_half, tcp_message.request_data, &uuid, name, state);
        Ok(())
    } else { 
        Err(MessageError::NotFound)
    }
}

async fn async_uuid_tcp_handle(am_read_half: AM<OwnedReadHalf>, am_write_half: AM<OwnedWriteHalf>, uuid: Uuid, main_write: ClientThreadMessageSender, thread_read: ClientMessageReceiver) {
    let mut buffer = vec![0; 1024];
    println!("new client! uuid: {}", uuid);
    
    let mut read_half = am_read_half.lock().await;
    
    let name = match name_check(&mut read_half, &mut buffer).await {
        Ok(name) => {
            println!("setting name: {}", name);
            let mut send_data = Map::new();
            send_data.insert("name".to_string(), json!(name));
            
            let tcp_message = TcpMessage {
                send_type: "ok".to_string(),
                command: "name set".to_string(),
                uuid: uuid.to_string(),
                send_data,
                request_data: Map::new()
            };
            
            let mut write_half = am_write_half.lock().await;
            send_tcp_message(&mut write_half, tcp_message).await;
            
            name
        },
        Err(err) => {
            eprintln!("err: {}", err);
            
            let mut send_data = Map::new();
            send_data.insert("error message".to_string(), json!("can not setting name"));
            
            let mut request_data = Map::new();
            request_data.insert(uuid.to_string(), json!("name"));
            
            let tcp_message = TcpMessage {
                send_type: "error".to_string(),
                command: "warm".to_string(),
                uuid: uuid.to_string(),
                send_data,
                request_data
            };
            
            let mut write_half = am_write_half.lock().await;
            send_tcp_message(&mut write_half, tcp_message).await;
            String::new()
        }
    };
    
    let mut state = State::default();
    let mut write_half = am_write_half.lock().await;
    
    loop {
        let result = message_handle(&mut read_half, &mut write_half, uuid, &name, &mut state, &mut buffer).await;
        if result.is_err() {
            eprintln!("err!");
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
                
                // Uuid 확인. 성공, 실패시 메시지 보냄.
                let ((am_read_half, am_write_half), uuid) = match uuid_check(stream, &mut buffer).await {
                    Ok(((am_read_half, am_write_half), uuid)) => {
                        let mut write_half = am_write_half.lock().await;
                        
                        println!("am tcp stream is ok!");
                        
                        let tcp_message = TcpMessage {
                            send_type: "ok".to_string(),
                            command: "set_uuid".to_string(),
                            uuid: uuid.to_string(),
                            send_data: Map::new(),
                            request_data: Map::new()
                        };
                        
                        send_tcp_message(&mut write_half, tcp_message).await;
                        
                        drop(write_half);
                        
                        ((am_read_half, am_write_half), uuid)
                    },
                    Err((err, _, mut write_half)) => {
                        eprintln!("err: {}", err);
                        
                        let tcp_message = TcpMessage {
                            send_type: "error".to_string(),
                            command: "fatal".to_string(),
                            uuid: String::new(),
                            send_data: Map::new(),
                            request_data: Map::new()
                        };
                        
                        send_tcp_message(&mut write_half, tcp_message).await;
                        
                        return;
                    }
                };

                let (thread_write, thread_read) = unbounded_channel();
                
                thread_write_map.insert(uuid, thread_write);

                let main_thread_write = main_thread_write.clone();

                spawn(async move {
                    async_uuid_tcp_handle(am_read_half, am_write_half, uuid, main_thread_write, thread_read).await
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
