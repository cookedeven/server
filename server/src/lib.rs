use std::{
    fs::File,
    io::Read,
    panic,
    str::{self, FromStr}, sync::Arc
};
use std::ops::Deref;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
        TcpStream
    },
    spawn,
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        Mutex,
        RwLock
    },
    task::JoinSet
};
use uuid::Uuid;
use dashmap::DashMap;
use lazy_static::lazy_static;
use serde_json::{json, to_vec, Map, Value};
use libCode::*;

pub type AM<T> = Arc<Mutex<T>>;
pub type ARL<T> = Arc<RwLock<T>>;
pub type Name = String;
pub type PlayerInfo = (Uuid, Name);
pub type PlayerQueue = (Uuid, Name);
pub type PlayerData = Arc<DashMap<Uuid, Arc<DashMap<String, Value>>>>;
pub type SessionID = i64;

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

macro_rules! end_of_tasks {
    ($arc_mutex_vec_unbounded_sender_thread_message:expr) => {
        $arc_mutex_vec_unbounded_sender_thread_message.lock().await.iter().for_each(|sender| {
            let _ = sender.send(ThreadMessage::Command(ThreadMessageCommand::EndOfTask));
        });
    };
}

lazy_static! {
    static ref PLAYER_TABLE: Arc<DashMap<Uuid, (AM<OwnedReadHalf>, AM<OwnedWriteHalf>)>> = ad!();
    static ref MATCH_QUEUE_2: AM<Vec<Uuid>> = am!(Vec::new());
    static ref MATCH_QUEUE_4: AM<Vec<Uuid>> = am!(Vec::new());
    static ref USER_DATA: PlayerData = ad!();
    static ref USER_SESSION_DATA: Arc<DashMap<SessionID, PlayerData>> = ad!();
    static ref USER_UNBOUNDED_SENDERS: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>> = ad!();
    static ref USER_SERVER_UNBOUNDED_SENDERS: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>> = ad!();
    static ref SERVER_UNBOUNDED_SENDERS: Arc<DashMap<SessionID, UnboundedSender<ThreadMessage>>> = ad!();
}

#[derive(Default, Eq, PartialEq)]
struct UserState {
    data: UserData,
    uuid: Uuid,
    matching_state: MatchingState
}

impl UserState {
    pub fn new(uuid: Uuid) -> Self {
        Self { uuid, ..Default::default() }
    }
}

enum ThreadMessageData {
    String(String),
    UserState(UserState),
    SessionID(SessionID),
    AnyData(Box<dyn Send + Sync>)
}

impl PartialEq<Self> for ThreadMessageData {
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (ThreadMessageData::String(_), ThreadMessageData::String(_)) | (ThreadMessageData::UserState(_), ThreadMessageData::UserState(_)) | (ThreadMessageData::AnyData(_), ThreadMessageData::AnyData(_)))
    }
}

#[derive(PartialEq)]
enum ThreadMessageCommand {
    NewConnect,
    NewPlayer(Uuid),
    EndOfConnect(Uuid),
    AddPlayer(Uuid),
    NewSession2(Uuid, Uuid),
    NewSession4(Uuid, Uuid, Uuid, Uuid),
    EndOfTask,
}

#[derive(PartialEq)]
pub enum ThreadMessage {
    Data(ThreadMessageData),
    Command(ThreadMessageCommand),
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
                let mut player_table = &PLAYER_TABLE;
                let am_read_write_half = (am!(read_half), am!(write_half));
                player_table.insert(uuid, am_read_write_half.clone());
                Ok((am_read_write_half, uuid))
            }
            "check" => {
                let uuid = match Uuid::from_str(&tcp_message.uuid) {
                    Ok(uuid) => uuid,
                    Err(_) => return Err((MessageError::InvalidUUID, read_half, write_half)),
                };
                let player_table = &PLAYER_TABLE;
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

fn send_data_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, send_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Result<(), MessageError> {
    if send_data.is_empty() {
        return Ok(())
    }

    for (uuid, data_value) in send_data {
        match data_value {
            Value::String(data_name) => {
                let _ = match data_name.as_str() {
                    "into_matching" => {
                        match state.matching_state {
                            MatchingState::Nothing => state.matching_state = MatchingState::Matching,
                            _ => continue
                        }
                    }
                    "out_matching" => {
                        match state.matching_state {
                            MatchingState::Matching => state.matching_state = MatchingState::Nothing,
                            _ => continue
                        }
                    }
                    _ => continue
                };
            }
            _ => return Err(MessageError::InvalidType)
        }
    }

    Ok(())
}

fn request_data_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, request_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Result<(), MessageError> {
    if request_data.is_empty() {
        return Ok(())
    }

    for (uuid, data_value) in &request_data {
        match data_value {
            Value::String(data_name) => {
                let _ = match data_name.as_str() {
                    "a" => "",
                    _ => continue
                };
            }
            _ => return Err(MessageError::InvalidType)
        }
    }
    
    Ok(())
}

async fn message_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, uuid: Uuid, name: &Name, state: &mut UserState, buffer: &mut [u8]) -> Result<(), MessageError> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

    if tcp_message.send_type.as_str() == "connect" {
        let send_data_result = send_data_handle(read_half, write_half, tcp_message.send_data, &uuid, name, state);
        let request_data_result = request_data_handle(read_half, write_half, tcp_message.request_data, &uuid, name, state);

        match (send_data_result, request_data_result) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(e1), Ok(_)) => Err(e1),
            (Ok(_), Err(e2)) => Err(e2),
            (Err(e1), Err(e2)) => Err(MessageError::Errors(vec![Box::new(e1), Box::new(e2)])),
        }
    } else { 
        Err(MessageError::NotFound)
    }
}

async fn player_handle(am_read_half: AM<OwnedReadHalf>, am_write_half: AM<OwnedWriteHalf>, uuid: Uuid, main_write: UnboundedSender<ThreadMessage>, thread_read: UnboundedReceiver<ThreadMessage>, mut buffer: Vec<u8>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>) -> Result<Uuid, (Uuid, MessageError)> {
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

    drop(read_half);

    let read_half_clone_0 = am_read_half.clone();
    let mut state = UserState::default();
    let mut tasks = JoinSet::new();
    let write_half_clone_0 = am_write_half.clone();
    let tasks_sender = am!(Vec::new());
    let tasks_sender_clone_0 = tasks_sender.clone();
    let tasks_sender_clone_1 = tasks_sender.clone();

    tasks.spawn(async move {
        let mut write_half = write_half_clone_0.lock().await;
        let mut read_half = read_half_clone_0.lock().await;
        let (sender, mut receiver) = unbounded_channel();
        tasks_sender_clone_0.lock().await.push(sender);
        drop(tasks_sender_clone_0);
        loop {
            let result = message_handle(&mut read_half, &mut write_half, uuid, &name, &mut state, &mut buffer).await;
            if let Err(error) = result {
               if error.unrecoverable_error() {
                   return Err(MessageError::OtherError(error.into()));
               }
            }

            if let Some(message) = receiver.recv().await {
                match message {
                    ThreadMessage::Data(ThreadMessageData::SessionID(session_id)) => {
                        if state.matching_state == MatchingState::Matching {

                        } else {
                            let session_senders = SERVER_UNBOUNDED_SENDERS.clone();
                            let Some(session_sender) = session_senders.get(&session_id) else {
                                return Err(MessageError::NotFound)
                            };

                        }
                    }
                    ThreadMessage::Data(_) => {}
                    ThreadMessage::Command(ThreadMessageCommand::EndOfTask) => return Ok(()),
                    ThreadMessage::Command(_) => {}
                }
            }
        }
    });

    if let Some(result) = tasks.join_next().await {
        match result {
            Ok(task_result) => {
                match task_result {
                    Ok(()) => {
                        end_of_tasks!(tasks_sender);
                        Ok(uuid)
                    },
                    Err(error) => {
                        end_of_tasks!(tasks_sender);
                        Err((uuid, error))
                    },
                }
            },
            Err(err) => {
                end_of_tasks!(tasks_sender);
                Err((uuid, MessageError::OtherError(err.into())))
            }
        }
    } else {
        end_of_tasks!(tasks_sender);
        Err((uuid, MessageError::UndefinedError))
    }
}

async fn user_manager(mut thread_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>) {

}

async fn session_manager_2(session_id: SessionID, session_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>, uuid_1: Uuid, uuid_2: Uuid) -> Result<(), MessageError> {
    let user_senders = USER_UNBOUNDED_SENDERS.clone();
    let Some(user_1_sender) = user_senders.get(&uuid_1) else {
        return Err(MessageError::NotFound);
    };

    let Some(user_2_sender) = user_senders.get(&uuid_2) else {
        return Err(MessageError::NotFound);
    };

    let _ = user_1_sender.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)));
    let _ = user_2_sender.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_2, uuid_1)));

    let _ = user_1_sender.send(ThreadMessage::Data(ThreadMessageData::SessionID(session_id)));
    let _ = user_2_sender.send(ThreadMessage::Data(ThreadMessageData::SessionID(session_id)));




    Ok(())
}

async fn session_manager_4(session_id: SessionID, session_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>, uuid_1: Uuid, uuid_2: Uuid, uuid_3: Uuid, uuid_4: Uuid) {

}

pub async fn player_tcp_handle(mut server_read: UnboundedReceiver<ThreadMessage>, server_write: UnboundedSender<ThreadMessage>) {
    let Ok(listener) = TcpListener::bind(SERVER_IP).await else {
        panic!("cannot bind {}", SERVER_IP)
    };
    println!("Server listening on {}", SERVER_IP);

    let (main_thread_write, mut main_thread_read) = unbounded_channel();
    let mut thread_write_map = ad!();
    let mut tasks = JoinSet::new();
    let server_write_clone_0 = server_write.clone();
    let server_write_clone_1 = server_write.clone();
    let server_write_clone_2 = server_write.clone();

    spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("new client!!!!!!! addr: {}", addr);
                    let server_message = ThreadMessage::Command(ThreadMessageCommand::NewConnect);
                    let _ = server_write_clone_0.send(server_message);
                    let mut buffer = vec![0; 8192];

                    // Uuid 확인. 성공, 실패시 메시지 보냄.
                    let (am_read_half, am_write_half, uuid) = match uuid_check(stream, &mut buffer).await {
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

                            (am_read_half, am_write_half, uuid)
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

                    let _ = server_write_clone_0.send(ThreadMessage::Command(ThreadMessageCommand::NewPlayer(uuid)));

                    // 스레드 채널 생성
                    let (thread_write, thread_read) = unbounded_channel();
                    // 스레드 채널 추가
                    thread_write_map.insert(uuid, thread_write);
                    // 메인 채널 복사
                    let main_thread_write = main_thread_write.clone();
                    // 스레드 채널 참조 복사
                    let thread_write_map_clone = thread_write_map.clone();

                    tasks.spawn(async move {
                        player_handle(am_read_half, am_write_half, uuid, main_thread_write, thread_read, buffer, thread_write_map_clone).await
                    });
                }
                Err(err) => eprintln!("tcp server err: {}", err)
            }

            if let Some(res) = tasks.try_join_next() {
                match res {
                    Ok(result) => match result {
                        Ok(uuid) => {
                            thread_write_map.remove(&uuid);
                            let _ = server_write.send(ThreadMessage::Command(ThreadMessageCommand::EndOfConnect(uuid)));
                        }
                        Err((uuid, err)) => {
                            thread_write_map.remove(&uuid);
                            eprintln!("err: {}", err);
                        }
                    }
                    Err(err) => {
                        eprintln!("task panic!: {}", err);
                        thread_write_map.retain(|_, sender| !sender.is_closed());
                    }
                }
            }
        }
    });

    spawn(
        async move {
            loop {
                let Some(read) = server_read.recv().await else {
                    continue
                };


            }
        }
    );

    spawn(
        async move {
            loop {
                let mut match_queue_2 = MATCH_QUEUE_2.lock().await;
                let mut match_queue_4 = MATCH_QUEUE_4.lock().await;
                if match_queue_2.len() >= 2 {
                    let uuid_1 = match_queue_2.remove(0);
                    let uuid_2 = match_queue_2.remove(0);
                    let _ = server_write_clone_2.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)));
                }

                if match_queue_4.len() >= 4 {
                    let uuid_1 = match_queue_4.remove(0);
                    let uuid_2 = match_queue_4.remove(0);
                    let uuid_3 = match_queue_4.remove(0);
                    let uuid_4 = match_queue_4.remove(0);
                    let _ = server_write_clone_2.send(ThreadMessage::Command(ThreadMessageCommand::NewSession4(uuid_1, uuid_2, uuid_3, uuid_4)));
                }
            }
        }
    );
}

pub async fn server_loop(mut clients_read: UnboundedReceiver<ThreadMessage>, client_write: UnboundedSender<ThreadMessage>) {
    let (main_thread_write, mut main_thread_read) = unbounded_channel();
    let tcp_thread_senders = USER_UNBOUNDED_SENDERS.clone();
    let user_thread_senders = USER_SERVER_UNBOUNDED_SENDERS.clone();
    let session_thread_senders = SERVER_UNBOUNDED_SENDERS.clone();
    let match_queue_2 = MATCH_QUEUE_2.clone();
    let match_queue_4 = MATCH_QUEUE_4.clone();
    let mut tasks = JoinSet::new();
    let mut sessions = JoinSet::new();
    let mut sessions_id = 0;

    loop {
        let Some(read) = clients_read.recv().await else {
            continue
        };



        match read {
            ThreadMessage::Data(data) => {
                todo!()
            }
            ThreadMessage::Command(command) => {
                match command { 
                    ThreadMessageCommand::NewConnect => println!("new client!"),
                    ThreadMessageCommand::NewPlayer(uuid) => {
                        user_thread_senders.entry(uuid).or_insert_with(|| {
                            let (sender, receiver) = unbounded_channel();
                            let tcp_thread_senders_clone = tcp_thread_senders.clone();
                            let main_thread_write_clone = main_thread_write.clone();
                            tasks.spawn(async move {
                                user_manager(receiver, main_thread_write_clone, tcp_thread_senders_clone).await;
                            });
                            sender
                        });
                    }
                    ThreadMessageCommand::EndOfConnect(uuid) => {
                        let Some((_uuid, sender)) = user_thread_senders.remove(&uuid) else {
                            continue
                        };

                        let _ = sender.send(ThreadMessage::Command(ThreadMessageCommand::EndOfConnect(uuid)));
                    }
                    ThreadMessageCommand::NewSession2(uuid_1, uuid_2) => {
                        let (sender, receiver) = unbounded_channel();
                        if session_thread_senders.insert(sessions_id, sender).is_none() {
                            let tcp_thread_senders_clone = tcp_thread_senders.clone();
                            let main_thread_write_clone = main_thread_write.clone();
                            sessions.spawn(async move {
                                session_manager_2(sessions_id, receiver, main_thread_write_clone, tcp_thread_senders_clone, uuid_1, uuid_2).await;
                            });
                            sessions_id += 1;
                        }
                    }
                    ThreadMessageCommand::NewSession4(uuid_1, uuid_2, uuid_3, uuid_4) => {
                        let (sender, receiver) = unbounded_channel();
                        if session_thread_senders.insert(sessions_id, sender).is_none() {
                            let tcp_thread_senders_clone = tcp_thread_senders.clone();
                            let main_thread_write_clone = main_thread_write.clone();
                            sessions.spawn(async move {
                                session_manager_4(sessions_id, receiver, main_thread_write_clone, tcp_thread_senders_clone, uuid_1, uuid_2, uuid_3, uuid_4).await;
                            });
                            sessions_id += 1;
                        }
                    }
                    _ => todo!()
                }
            }
        }
    }
}

pub fn init(file: &mut File) -> Result<(), MessageError> {
    let mut buffer = String::new();
    file.read_to_string(&mut buffer)?;

    Ok(())
}
