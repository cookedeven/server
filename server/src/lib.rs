use std::{
    fmt::Display,
    sync::Arc,
    ops::Range,
    hash::Hash
};
use tokio::{
    net::{TcpListener, TcpStream},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, OnceCell, MutexGuard},
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;
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
    static ref PLAYER_DATA: AM<Vec<PlayerStream>> = am!(Vec::new());
    static ref PLAYER_PLAY: AM<Vec<(PlayerStream, PlayerStream, PlayerStream, PlayerStream)>> = am!(Vec::new());
    static ref TCP_LISTENER: OnceCell<AM<TcpListener>> = OnceCell::new();
}

const COMMAND_POS: usize = 0;
const UUID_POS: usize = 1;
const ID_POS: usize = 1;

fn search_id<T: PartialEq, U>(data: &[(T, U)], target: T) -> Option<usize> {
    data.iter().position(|(a, _)| *a == target)
}

async fn tcp_get_uuid(message: &[&str], stream: &mut TcpStream) -> Result<(), MessageError> {
    
    let id: usize = match message.get(ID_POS) { 
        Some(id_str) => {
            match id_str.parse() {
                Ok(id) => id,
                Err(err) => return Err(MessageError::OtherError(err.into()))
            }
        }
        None => return Err(MessageError::NotLongEnough)
    };

    if let Some((uuid, name)) = PLAYER_DATA.lock().await.get(id) {
        let uuid_name_chain: Vec<u8> = uuid.as_bytes().into_iter().cloned().chain(name.as_bytes().iter().cloned()).collect();
        let _ = stream.write_all(&uuid_name_chain).await;
        Ok(())
    } else {
        let _ = stream.write_all(b"invalid id").await;
        Ok(())
    }
}

async fn tcp_new_uuid(message: &[&str], stream: &mut TcpStream) -> Result<(), MessageError> {
    let uuid = Uuid::new_v4();
    let new_name: Name = match message.get(UUID_POS) {
        Some(name) => name,
        None => return Err(MessageError::NotLongEnough)
    }.to_string();
    let uuid_string = uuid.to_string();
    PLAYER_DATA.lock().await.push((uuid, new_name));
    let _ = stream.write_all(&uuid_string.as_bytes()).await;
    Ok(())
}


async fn tcp_connect_uuid(message: &[&str], stream: &mut TcpStream) -> Result<(), MessageError> {
    let uuid_string = match message.get(UUID_POS) {
        Some(uuid) => uuid,
        None => return Err(MessageError::NotLongEnough)
    };
    
    let uuid = match Uuid::from_str(uuid_string) {
        Ok(uuid) => uuid,
        Err(err) => return Err(MessageError::OtherError(err.into()))
    };
    
    println!("{}", message[2]);

    let data = &PLAYER_DATA.lock().await;
    
    if let Some(_) = search_id(data, uuid){
        let _ = stream.write_all(format!("connect to UUID! uuid: {}", uuid).as_bytes()).await;
    } else {
        let _ = stream.write_all(b"invalid UUID").await;
    }
    Ok(())
}

async fn async_uuid_tcp_handle(mut stream: TcpStream) {
    let mut buffer = vec![0; 1024];
    println!("new client!: {:?}", stream);
    match stream.read(&mut buffer).await {
        Ok(size) => {
            let message = &buffer[..size];
            let string_message = String::from_utf8(message.to_owned()).unwrap();

            let split_data: Vec<_> = string_message.split(".").collect();
                
            println!("{:?}", split_data);

            let result = match split_data.get(COMMAND_POS) {
                Some(command) => {
                    match *command {
                        "END" => return,
                        "GET_UUID" => tcp_get_uuid(&split_data, &mut stream).await,
                        "NEW_UUID" => tcp_new_uuid(&split_data, &mut stream).await,
                        "CONNECT_UUID" => tcp_connect_uuid(&split_data, &mut stream).await,
                        _ => Err(MessageError::CommandNotFound)
                    }
                }
                None => Err(MessageError::NotFound)
            };
            /*
            if let Err(err) = result {
                eprintln!("err: {}", err);
            }
            */
        }
        Err(err) => eprintln!("err: {}", err) 
    };
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

/*
async fn uuid_udp_parse_input(uuid: [u8; 16], message: &[u8]) -> Result<(), MessageError> {
    let uuid = Uuid::from_bytes(uuid);
    let Some(id) = search_id(&PLAYER_DATA.lock().await, uuid) else {
        return Err(MessageError::NotFound)
    };

    let (uuid, name) = match PLAYER_DATA.lock().await.get(id) {
        Some(container) => container,
        None => return Err(MessageError::NotFound)
    };



    todo!()
}

async fn uuid_udp_handle(message: &[u8]) -> Result<(), MessageError> {
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
    return Ok(())
}


pub async fn uuid_udp_server() {
    if UDP_SOCKET.get().is_none() {
        init_udp_socket().await;
    }
    
    let arc_mutex_socket = match UDP_SOCKET.get() {
        Some(socket) => socket,
        None => panic!()
    };
    
    let socket = arc_mutex_socket.lock().await;

    let mut buf = vec![0; 1024];
    loop {
        let error = match socket.recv_from(&mut buf).await {
            Ok((size, _addr)) => {
                let message = &buf[..size];
                uuid_udp_handle(message).await
            }
            Err(err) => Err(MessageError::OtherError(err.into())),
        };
        if let Err(err) = error {
            let logging = ErrorLogging::new(Box::new(err), SystemTime::now(), PROGRAM_TIME.elapsed());
            let log = MessageLog::new(Box::new(logging));
            LOGGING_DATA.lock().await.push(log)
        }
    }
}
*/

use std::io::{Read, Write};
use std::ptr;
use std::ffi::CString;
use std::str::FromStr;
use anyhow::Error;
use tokio::runtime::Runtime;

// 서버 핸들 정의: TcpListener를 감싸는 구조체
pub struct ServerHandle {
    pub listener: TcpListener,
}

// 클라이언트 핸들 정의: TcpStream을 감싸는 구조체
pub struct ClientHandle {
    pub stream: TcpStream,
}

#[no_mangle]
pub extern "C" fn server_create() -> *mut ServerHandle {
    let rt = Runtime::new().unwrap();
    
    let listener = rt.block_on(async {
        TcpListener::bind(SERVER_IP).await.unwrap()
    });

    // 서버 핸들을 박스에 담고, 포인터로 변환하여 반환
    let handle = Box::into_raw(Box::new(ServerHandle { listener }));
    
    handle
}

#[no_mangle]
pub extern "C" fn server_shutdown(handle: *mut ServerHandle) {
    if !handle.is_null() {
        unsafe {
            // 서버 핸들을 이용해 리스너 종료
            let server = Box::from_raw(handle);
            // 서버 종료 작업 (리소스 자동 해제)
            drop(server);
        }
    }
}
