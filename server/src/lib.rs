use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering}
    },
    hash::Hash,
    collections::HashMap,
    str::FromStr
};
use std::error::Error;
use std::f32::consts::E;
use std::fmt::Display;
use tokio::{
    net::{TcpListener, TcpStream},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, OnceCell},
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;
use tokio::io::{ReadHalf, split, WriteHalf};
use tokio::sync::MutexGuard;
use libCode::{MessageError, SERVER_IP};

type AM<T> = Arc<Mutex<T>>;
type Name = String;
type PlayerStream = (Uuid, Name);

macro_rules! am {
    ($t:expr) => {
        Arc::new(Mutex::new($t))
    };
}

lazy_static! {
    static ref PLAYER_TABLE: AM<HashMap<Uuid, AM<TcpStream>>> = am!(HashMap::new());
    static ref MATCH_QUEUE_2: AM<Vec<[PlayerStream; 2]>> = am!(Vec::new());
    static ref MATCH_QUEUE_4: AM<Vec<[PlayerStream; 4]>> = am!(Vec::new());
    static ref PLAYER_DATA: AM<Vec<PlayerStream>> = am!(Vec::new());
    static ref PLAYER_QUEUE: AM<Vec<AM<PlayerStream>>> = am!(Vec::new());
}

const COMMAND_POS: usize = 0;
const UUID_POS: usize = 1;
const ID_POS: usize = 1;
const NAME_POS: usize = 1;
const CONNECT_TYPE: usize = 2;
const MATCH_COMMAND_POS: usize = 2;

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
    let uuid_string = match message.get(UUID_POS) {
        Some(uuid) => uuid,
        None => return Err(MessageError::NotLongEnough)
    };
    
    let uuid = match Uuid::parse_str(uuid_string) {
        Ok(uuid) => uuid,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };
    
    let _ = match message.get(CONNECT_TYPE) {
        Some(&data) => {
            match data { 
                "MATCHING" => tcp_match(uuid, Arc::clone(&am_tcp_stream)).await,
                _ => Err(MessageError::UndefinedError)
            }
        }
        None => Err(MessageError::NotLongEnough)
    };
    
    println!("{}", message[2]);
        let mut tcp_stream = am_tcp_stream.lock().await;
    let data = &PLAYER_DATA.lock().await;
    
    if let Some(_) = search_id(data, uuid){
        let _ = tcp_stream.write_all(format!("connect to UUID! uuid: {}", uuid).as_bytes()).await;
    } else {
        let _ = tcp_stream.write_all(b"invalid UUID").await;
    }
    Ok(())
}

async fn match_handle(message: &[&str], tcp_stream: AM<TcpStream>, uuid: Uuid, wait: &mut bool, playing: &mut bool) -> Result<(), MessageError> {
    let player_queue = PLAYER_QUEUE.lock().await;
    match message.get(MATCH_COMMAND_POS) {
        Some(&"JOIN") => {
            player_queue.push()
            *wait = true;
        }
        Some(&"LEAVE") => {
            *wait = false;
        }
        Some(_) => {}
        None => {}
    }

    Ok(())
}

async fn connect_handle(message: &[&str], am_tcp_stream: AM<TcpStream>) -> Result<(), MessageError> {
    match message.get(COMMAND_POS) {
        Some(&command) => {
            match command {
                "END" => return Ok(()),
                "CONNECT_UUID" => tcp_connect_uuid(&message, am_tcp_stream).await,
                "RETURN" => {
                    let mut tcp_stream = am_tcp_stream.lock().await;
                    let _ = tcp_stream.write_all(b"RETURN").await;
                    Ok(())
                },
                _ => Err(MessageError::CommandNotFound)
            }
        }
        None => Err(MessageError::NotFound)
    }
}

async fn uuid_check(mut tcp_stream: TcpStream) -> Result<(AM<TcpStream>, Uuid), (MessageError, TcpStream)> {
    let mut buffer = vec![0; 1024];
    match tcp_stream.read(&mut buffer).await {
        Ok(size) => {
            let message = &buffer[..size];
            let string_message = String::from_utf8_lossy(message);
            let split_message: Vec<&str> = string_message.split(".").collect();

            match split_message.get(COMMAND_POS) {
                Some(&"NEW") => {
                    let uuid = Uuid::new_v4();
                    let mut player_table = PLAYER_TABLE.lock().await;
                    let am_tcp_stream = am!(tcp_stream);
                    player_table.insert(uuid, Arc::clone(&am_tcp_stream));
                    Ok((Arc::clone(&am_tcp_stream), uuid))
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

async fn loop_handle(tcp_stream: &mut TcpStream, am_tcp_stream: &AM<TcpStream>, buffer: &mut Vec<u8>, uuid: Uuid, wait: &mut bool, play: &mut bool) -> Result<(), MessageError> {
    match tcp_stream.read(buffer).await {
        Ok(size) => {
            let message = &buffer[..size];

            let string_message = match String::from_utf8(message.into()) {
                Ok(string_message) => string_message,
                Err(err) => return Err(MessageError::OtherError(err.into()))
            };

            let split_data: Vec<_> = string_message.split(".").collect();

            println!("{:?}", split_data);

            let Some(command) = split_data.get(COMMAND_POS).copied() else {
                return Err(MessageError::NotFound)
            };

            match command {
                "MATCH" => match_handle(&split_data, Arc::clone(am_tcp_stream), uuid, wait, play).await,
                "CONNECT" => connect_handle(&split_data, Arc::clone(am_tcp_stream)).await,
                _ => Err(MessageError::NotFound)
            }
        }
        Err(err) => Err(MessageError::OtherError(err.into()))
    }
}

async fn async_uuid_tcp_handle(tcp_stream: TcpStream) {
    let mut buffer = vec![0; 1024];
    println!("new client!");

    let (am_tcp_stream, uuid) = match uuid_check(tcp_stream).await {
        Ok(am_tcp_stream) => am_tcp_stream,
        Err((err, mut tcp_stream)) => {
            eprintln!("err: {}", err);
            let _ = tcp_stream.write_all(format!("err: {}\n", err).as_bytes()).await;
            return;
        }
    };
    
    let mut tcp_stream = am_tcp_stream.lock().await;
    
    let mut wait_matching = false;
    let mut play_matching = false;
    loop {
        let result = loop_handle(&mut tcp_stream, &am_tcp_stream, &mut buffer, uuid, &mut wait_matching, &mut play_matching).await;
        
        if let Err(err) = result {
            eprintln!("err: {}", err);
        }
    }
}

async fn read_uuid_from_stream<E: Error>(p0: &mut TcpStream) -> Result<Uuid, E> {
    todo!()
}

pub async fn uuid_tcp_server() {
    let listener = TcpListener::bind(SERVER_IP).await.unwrap();
    println!("Server listening on {}", SERVER_IP);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                spawn(async move {
                    async_uuid_tcp_handle(stream).await
                });
            }
            Err(err) => eprintln!("tcp server err: {}", err)
        }
    }
}
