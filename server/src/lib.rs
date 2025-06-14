use std::{
    sync::Arc,
    collections::HashMap,
    str::{self, FromStr}
};
use std::error::Error;
use std::fmt::{Display, format};
use tokio::{
    net::{TcpListener, TcpStream},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        Mutex,
    },
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use libCode::{MessageError, SERVER_IP, QueueType};

type AM<T> = Arc<Mutex<T>>;
type Name = String;
type PlayerInfo = (Uuid, Name);
type PlayerQueue = (Uuid, Name, QueueType);
type MessageSender = UnboundedSender<Message>;
type MessageReceiver = UnboundedReceiver<Message>;

macro_rules! am {
    ($t:expr) => {
        Arc::new(Mutex::new($t))
    };
}

lazy_static! {
    static ref PLAYER_TABLE: AM<HashMap<Uuid, AM<TcpStream>>> = am!(HashMap::new());
    static ref MATCH_QUEUE_2: AM<Vec<[PlayerInfo; 2]>> = am!(Vec::new());
    static ref MATCH_QUEUE_4: AM<Vec<[PlayerInfo; 4]>> = am!(Vec::new());
    static ref PLAYER_DATA: AM<Vec<PlayerInfo>> = am!(Vec::new());
    static ref PLAYER_QUEUE: AM<Vec<PlayerQueue>> = am!(Vec::new());
}

const COMMAND_POS: usize = 0;
const SECOND_COMMAND_POST: usize = 1;
const UUID_POS: usize = 1;
const CONNECT_UUID_POS: usize = 2;
const NAME_POS: usize = 1;
const CONNECT_TYPE: usize = 3;
const MATCH_COMMAND_POS: usize = 2;
const MATCH_QUEUE_TYPE_POS: usize = 3;

struct Message {
    uuid: Uuid,
    context: String
}

impl Message {
    fn new(uuid: Uuid, context: String) -> Self {
        Self { uuid, context }
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
            let mut player_queue = PLAYER_QUEUE.lock().await;
            //player_queue.push()
        }
        None => return Err(MessageError::NotFound)
    }

    Ok(())
}

async fn tcp_connect_uuid(message: &[&str], am_tcp_stream: AM<TcpStream>) -> Result<(), MessageError> {
    let uuid_string = match message.get(CONNECT_UUID_POS) {
        Some(uuid) => uuid,
        None => return Err(MessageError::NotLongEnough)
    };
    
    let uuid = match Uuid::parse_str(uuid_string) {
        Ok(uuid) => uuid,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };
    
    let _ = match message.get(CONNECT_TYPE) {
        Some(&"MATCHING") => tcp_match(uuid, Arc::clone(&am_tcp_stream)).await,
        Some(_) => Err(MessageError::UndefinedError),
        None => Err(MessageError::NotLongEnough)
    };
    
    println!("{}", message[2]);
    let mut tcp_stream = am_tcp_stream.lock().await;
    let data = &PLAYER_DATA.lock().await;
    
    if let Some(_) = search_id(data, uuid) {
        let _ = tcp_stream.write_all(format!("connect to UUID! uuid: {}", uuid).as_bytes()).await;
    } else {
        let _ = tcp_stream.write_all(b"invalid UUID").await;
    }
    Ok(())
}

async fn match_handle(message: &[&str], am_tcp_stream: AM<TcpStream>, uuid: Uuid, name: &Name, wait: &mut bool, playing: &mut bool) -> Result<(), MessageError> {
    let mut player_queue = PLAYER_QUEUE.lock().await;
    let mut tcp_stream = am_tcp_stream.lock().await;
    
    match message.get(MATCH_COMMAND_POS) {
        Some(&"JOIN") => {
            let queue_type = match message.get(MATCH_QUEUE_TYPE_POS) { 
                Some(&"1vs1") => Ok(QueueType::Two),
                Some(&"2vs2") => Ok(QueueType::Four),
                Some(_) => Err(MessageError::CommandNotFound),
                None => Err(MessageError::NotLongEnough)
            }?;
            player_queue.push((uuid, name.clone(), queue_type));
            let _ = tcp_stream.write_all(format!("MATCH.JOIN.{}", uuid).as_bytes());
            *wait = true;
        }
        Some(&"LEAVE") => {
            player_queue.retain(|(queue_uuid, _queue_name, _queue_type)| *queue_uuid != uuid);
            let _ = tcp_stream.write_all(format!("MATCH.LEAVE.{}", uuid).as_bytes());
            *wait = false;
        }
        Some(&other) if *playing => {
            todo!()
        }
        Some(_) => return Err(MessageError::CommandNotFound),
        None => return Err(MessageError::NotLongEnough)
    }

    Ok(())
}

async fn connect_handle(message: &[&str], am_tcp_stream: AM<TcpStream>) -> Result<(), MessageError> {
    match message.get(SECOND_COMMAND_POST) {
        Some(&"END") => return Ok(()),
        Some(&"UUID") => tcp_connect_uuid(&message, am_tcp_stream).await,
        Some(_) => Err(MessageError::CommandNotFound),
        None => Err(MessageError::NotFound)
    }
}

async fn uuid_check(mut tcp_stream: TcpStream, buffer: &mut [u8]) -> Result<(AM<TcpStream>, Uuid), (MessageError, TcpStream)> {
    match tcp_stream.read(buffer).await {
        Ok(size) => {
            let message = &buffer[..size];
            let str_message = match str::from_utf8(message) {
                Ok(message) => message,
                Err(err) => return Err((MessageError::OtherError(err.into()), tcp_stream))
            };
            let split_message: Vec<_> = str_message.split('.').collect();

            match split_message.get(COMMAND_POS) {
                Some(&"NEW") => {
                    let uuid = Uuid::new_v4();
                    let mut player_table = PLAYER_TABLE.lock().await;
                    let am_tcp_stream = am!(tcp_stream);
                    player_table.insert(uuid, Arc::clone(&am_tcp_stream));
                    Ok((am_tcp_stream, uuid))
                }
                Some(&"CONNECT") => {
                    let uuid_str = match split_message.get(UUID_POS) {
                        Some(&s) => s,
                        None => return Err((MessageError::MissingUUID, tcp_stream)),
                    };
                    let uuid = match Uuid::from_str(uuid_str) {
                        Ok(uuid) => uuid,
                        Err(_) => return Err((MessageError::InvalidUUID, tcp_stream)),
                    };
                    let player_table = PLAYER_TABLE.lock().await;
                    match player_table.get(&uuid) {
                        Some(stream) => Ok((Arc::clone(stream), uuid)),
                        None => Err((MessageError::NotFound, tcp_stream))
                    }
                }
                Some(_) => Err((MessageError::CommandNotFound, tcp_stream)),
                None => Err((MessageError::EmptyCommand, tcp_stream)),
            }
        }
        Err(err) => Err((MessageError::OtherError(err.into()), tcp_stream))
    }
}

async fn name_check(am_tcp_stream: AM<TcpStream>, buffer: &mut [u8]) -> Result<Name, MessageError> {
    let mut tcp_stream = am_tcp_stream.lock().await;
    match tcp_stream.read(buffer).await { 
        Ok(size) => {
            let message = &buffer[..size];
            let str_message = match str::from_utf8(message) {
                Ok(message) => message,
                Err(err) => return Err(MessageError::OtherError(err.into()))
            };
            let split_message: Vec<_> = str_message.split('.').collect();
            
            let command = split_message.get(COMMAND_POS).copied().ok_or(MessageError::NotLongEnough)?;
            if command != "NAME" {
                return Err(MessageError::CommandNotFound);
            }
            let name = split_message.get(NAME_POS).copied().ok_or(MessageError::NotLongEnough)?;
            Ok(name.into())
        }
        Err(err) => Err(MessageError::OtherError(err.into()))
    }
}

async fn message_handle(am_tcp_stream: AM<TcpStream>, uuid: Uuid, name: &Name, wait_matching: &mut bool, play_matching: &mut bool, buffer: &mut [u8]) -> Result<(), Box<dyn Error>> {
    let mut tcp_stream = am_tcp_stream.lock().await;
    
    let size = tcp_stream.read(buffer).await?;
    
    let message = &buffer[..size];
    let string_message = String::from_utf8(message.to_vec())?;
    let split_data: Vec<_> = string_message.split('.').collect();
    println!("{:?}", split_data);
    
    match split_data.get(COMMAND_POS) {
        Some(&"MATCH") => {
            let _ = match_handle(&split_data, Arc::clone(&am_tcp_stream), uuid, name, wait_matching, play_matching).await;
        }
        Some(&"CONNECT") => {
            let _ = connect_handle(&split_data, Arc::clone(&am_tcp_stream)).await;
        }
        Some(_) => {
            eprintln!("command not found");
        }
        None => {
            eprintln!("command missing");
        }
    }
    Ok(())
}

async fn async_uuid_tcp_handle(am_tcp_stream: AM<TcpStream>, uuid: Uuid, tx_to_server: MessageSender, mut rx_from_server: MessageReceiver) {
    let mut buffer = vec![0; 1024];
    println!("new client!");
    
    let name = match name_check(Arc::clone(&am_tcp_stream), &mut buffer).await {
        Ok(name) => name,
        Err(err) => {
            eprintln!("err: {}", err);
            let mut tcp_stream = am_tcp_stream.lock().await;
            let _ = tcp_stream.write_all(format!("err: {}\n", err).as_bytes()).await;
            let _ = tcp_stream.write_all(b"ConnectEnd").await;
            return;
        }
    };
    
    let mut tcp_stream = am_tcp_stream.lock().await;
    
    let mut wait_matching = false;
    let mut play_matching = false;
    loop {
        tokio::select! {
            _ = async {
                message_handle(Arc::clone(&am_tcp_stream), uuid, &name, &mut wait_matching, &mut play_matching, &mut buffer)
            } => {}
            
            Some(msg) = rx_from_server.recv() => {
                todo!()
            }
        }
    }
}

async fn read_uuid_from_stream<E: Error>(p0: &mut TcpStream) -> Result<Uuid, E> {
    todo!()
}

pub async fn uuid_tcp_server(server_tx: MessageSender, client_senders: AM<HashMap<Uuid, MessageSender>>) {
    let Ok(listener) = TcpListener::bind(SERVER_IP).await else {
        panic!("cannot bind {}", SERVER_IP)
    };
    println!("Server listening on {}", SERVER_IP);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let mut buffer = vec![0; 256];
                let (am_tcp_stream, uuid) = match uuid_check(stream, &mut buffer).await {
                    Ok(am_tcp_stream) => am_tcp_stream,
                    Err((err, mut tcp_stream)) => {
                        eprintln!("err: {}", err);
                        let _ = tcp_stream.write_all(format!("err: {}\n", err).as_bytes()).await;
                        let _ = tcp_stream.write_all(b"ConnectEnd").await;
                        return;
                    }
                };
                let server_tx_clone = server_tx.clone();
                let (client_tx, client_rx) = unbounded_channel();
                
                spawn(async move {
                    async_uuid_tcp_handle(am_tcp_stream, uuid, server_tx_clone, client_rx).await
                });
            }
            Err(err) => eprintln!("tcp server err: {}", err)
        }
    }
}

pub async fn server_loop(mut rx_from_clients: MessageReceiver, client_senders: AM<HashMap<Uuid, MessageSender>>) {
    loop {

    }
}
