use std::{
    fs::File,
    io::Read,
    str::FromStr,
    sync::Arc,
};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
        TcpStream
    },
    spawn,
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        Mutex,
    },
    task::JoinSet
};
use uuid::Uuid;
use dashmap::DashMap;
use lazy_static::lazy_static;
use serde_json::{json, Map, Value};
// use libCode::{ErrorLevel, MatchingState, MessageError, SERVER_IP, TcpMessage, UserData};
use libCode::*;


pub type Name = String;
pub type PlayerData = AD<Uuid, AD<String, Value>>;
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

macro_rules! name_get {
    ($player_data:expr, $get_uuid:expr, $write_half:expr, $uuid:expr) => {
        $player_data.get(&$get_uuid).and_then(|data| data.get("name").map(|name_ref| name_ref.value().clone())).and_then(|name| {
            match name {
                Value::String(name) => Some(name),
                _ => None
            }
        }).unwrap_or({
            let mut send_data = Map::new();
            send_data_setting!(send_data,
                [$uuid, ("error".to_string(), json!(format!("invalid uuid: {}", $get_uuid)))]
            );

            let message = TcpMessage {
                send_type: "error".to_string(),
                command: ErrorLevel::Warning.to_string(),
                uuid: $uuid.to_string(),
                send_data,
                request_data: Map::new()
            };

            send_tcp_message(&mut $write_half, message).await;
            "ERRORRRRRRRRRRRRRRRRRRRRRRRRRRRR".to_string()
        })
    }
}

lazy_static! {
    static ref PLAYER_TABLE: AD<Uuid, (AM<OwnedReadHalf>, AM<OwnedWriteHalf>)> = ad!();
    static ref MATCH_QUEUE_2: AM<Vec<Uuid>> = am!(Vec::new());
    static ref MATCH_QUEUE_4: AM<Vec<Uuid>> = am!(Vec::new());
    static ref USER_DATA: PlayerData = ad!();
    static ref USER_SESSION_DATA: AD<SessionID, PlayerData> = ad!();
    static ref USER_UNBOUNDED_SENDERS: AD<Uuid, UnboundedSender<ThreadMessage>> = ad!();
    static ref USER_SERVER_UNBOUNDED_SENDERS: AD<Uuid, UnboundedSender<ThreadMessage>> = ad!();
    static ref SERVER_UNBOUNDED_SENDERS: AD<SessionID, UnboundedSender<ThreadMessage>> = ad!();
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
        matches!((self, other),
            (ThreadMessageData::String(_), ThreadMessageData::String(_)) |
            (ThreadMessageData::UserState(_), ThreadMessageData::UserState(_)) |
            (ThreadMessageData::SessionID(_), ThreadMessageData::SessionID(_)) |
            (ThreadMessageData::AnyData(_), ThreadMessageData::AnyData(_))
        )
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

async fn uuid_check(tcp_stream: TcpStream, buffer: &mut [u8]) -> Result<((AM<OwnedReadHalf>, AM<OwnedWriteHalf>), Uuid), (MessageError, OwnedReadHalf, OwnedWriteHalf)> {
    let (mut read_half, write_half) = tcp_stream.into_split();
    
    let tcp_message = match read_data(&mut read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err((err, read_half, write_half))
    };

    if &tcp_message.send_type != "uuid" {
        return Err((MessageError::NotFound, read_half, write_half))
    }

    match tcp_message.command.as_str() {
        "get" => {
            let uuid = Uuid::new_v4();
            let player_table = &PLAYER_TABLE;
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
}

async fn name_check(read_half: &mut OwnedReadHalf, uuid: Uuid, buffer: &mut [u8]) -> Result<Name, MessageError> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

    if tcp_message.send_type != "name" {
        return Err(MessageError::CommandNotFound);
    }

    let data_value = tcp_message.send_data.get(&uuid.to_string()).ok_or_else(|| MessageError::NotFound)?;
    let data = data_value.as_object().ok_or_else(|| MessageError::InvalidType)?;
    let name_value = data.get("name").ok_or_else(|| MessageError::NotFound)?;
    let name = name_value.as_str().ok_or_else(|| MessageError::InvalidType)?;

    Ok(name.to_string())
}

fn send_data_handle(send_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Result<(), MessageError> {
    if send_data.is_empty() {
        return Ok(())
    }

    println!("send_data: {:?}", send_data);

    for (send_data_uuid, data_value) in send_data {
        println!("send_data_uuid: {:?}", send_data_uuid);

        let data = data_value.as_object().ok_or_else(|| MessageError::InvalidType)?;

        if send_data_uuid == uuid.to_string() {
            for (data_name, value) in data {
                println!("data_name: {:?}", data_name);
                match data_name.as_str() {
                    "matching" => {
                        println!("value: {:?}", value);
                        let match_state = value.as_bool().ok_or_else(|| MessageError::InvalidType)?;
                        if match_state {
                            if state.matching_state == MatchingState::Nothing || state.matching_state == MatchingState::None {
                                println!("매칭중!: {}", name);
                                state.matching_state = MatchingState::Matching;
                            }
                        } else {
                            if state.matching_state == MatchingState::Matching {
                                println!("매칭 종료!: {}", name);
                                state.matching_state = MatchingState::Nothing;
                            }
                        }
                    }
                    _ => continue
                }
            }
        }
    }

    Ok(())
}

fn request_data_handle(request_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Result<Option<TcpMessage>, MessageError> {
    if request_data.is_empty() {
        return Ok(None)
    }

    for (uuid, data_value) in &request_data {
        match data_value {
            Value::Object(data_map) => {
                let send_data = data_map.clone();
                let message = TcpMessage {
                    send_type: "data".to_string(),
                    command: "push".to_string(),
                    uuid: uuid.to_string(),
                    send_data,
                    request_data: Map::new()
                };

                return Ok(Some(message))
            }
            _ => return Err(MessageError::InvalidType)
        }
    }
    
    Ok(None)
}

async fn message_handle(read_half: &mut OwnedReadHalf, write_half: &mut OwnedWriteHalf, uuid: Uuid, name: &Name, state: &mut UserState, buffer: &mut [u8]) -> Result<(), MessageError> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

    if tcp_message.send_type.as_str() != "connect" {
        return Err(MessageError::CommandNotFound);
    }

    let send_data_result = send_data_handle(tcp_message.send_data, &uuid, name, state);
    let request_data_result = request_data_handle(tcp_message.request_data, &uuid, name, state);

    match (send_data_result, request_data_result) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(e1), Ok(_)) => Err(e1),
        (Ok(_), Err(e2)) => Err(e2),
        (Err(e1), Err(e2)) => Err(MessageError::Errors(vec![e1, e2])),
    }
}

async fn player_handle(am_read_half: AM<OwnedReadHalf>, am_write_half: AM<OwnedWriteHalf>, uuid: Uuid, main_write: UnboundedSender<ThreadMessage>, thread_read: UnboundedReceiver<ThreadMessage>, mut buffer: Vec<u8>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>) -> Result<Uuid, (Uuid, MessageError)> {
    println!("new player! uuid: {}", uuid);
    
    let mut read_half = am_read_half.lock().await;

    let name = match name_check(&mut read_half, uuid, &mut buffer).await {
        Ok(name) => {
            println!("setting name: {}", name);
            let mut send_data = Map::new();
            send_data_setting!(send_data,
                [uuid.to_string(), ("name".to_string(), json!(name))]
            );
            
            let tcp_message = TcpMessage {
                send_type: "name_set".to_string(),
                command: ErrorLevel::Ok.to_string(),
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
            let mut request_data = Map::new();
            send_data_setting!(send_data,
                [uuid.to_string(), ("error".to_string(), json!("can not setting name"))]
            );
            request_data_setting!(request_data,
                [uuid.to_string(), "name".to_string()]
            );
            
            let tcp_message = TcpMessage {
                send_type: "error".to_string(),
                command: ErrorLevel::Warning.to_string(),
                uuid: uuid.to_string(),
                send_data,
                request_data
            };
            
            let mut write_half = am_write_half.lock().await;
            send_tcp_message(&mut write_half, tcp_message).await;

            "ERRORRRRRRRRRRRRRRRRRRRRRRRRRRRR".to_string()
        }
    };

    drop(read_half);

    let read_half_clone_0 = am_read_half.clone();
    let mut state = UserState::default();
    let mut tasks = JoinSet::new();
    let tasks_sender = am!(Vec::new());
    let tasks_sender_clone = tasks_sender.clone();
    let (sender, mut receiver) = unbounded_channel();

    tasks.spawn(async move {
        let mut write_half = am_write_half.lock().await;
        let mut read_half = read_half_clone_0.lock().await;
        tasks_sender_clone.lock().await.push(sender);
        loop {
            let result = message_handle(&mut read_half, &mut write_half, uuid, &name, &mut state, &mut buffer).await;
            if let Err(error) = result {
               if error.unrecoverable_error() {
                   return Err(MessageError::OtherError(error.into()));
               }
            }

            if let Ok(message) = receiver.try_recv() {
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
                    ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)) => {
                        let player_data = USER_DATA.clone();
                        let name_1 = name_get!(player_data, uuid_1, write_half, uuid.to_string());
                        let name_2 = name_get!(player_data, uuid_2, write_half, uuid.to_string());

                        let mut send_data = Map::new();
                        send_data_setting!(send_data,
                            [uuid_1.to_string(), ("name".to_string(), json!(name_1))],
                            [uuid_2.to_string(), ("name".to_string(), json!(name_2))]
                        );

                        let message = TcpMessage {
                            send_type: "data".to_string(),
                            command: "push".to_string(),
                            uuid: uuid.to_string(),
                            send_data,
                            request_data: Map::new()
                        };

                        send_tcp_message(&mut write_half, message).await;
                    }
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
    let thread_write_map = ad!();

    loop {
        let server_write_clone_0 = server_write.clone();
        let server_write_clone_1 = server_write.clone();
        let server_write_clone_2 = server_write.clone();
        let main_thread_write_clone_0 = main_thread_write.clone();
        let thread_write_map_clone_0 = thread_write_map.clone();
        let thread_write_map_clone_1 = thread_write_map.clone();
        let mut tasks = JoinSet::new();

        match listener.accept().await {
            Ok((stream, addr)) => {
                tasks.spawn(async move {
                    println!("new client!!!!!!! addr: {}", addr);
                    let server_message = ThreadMessage::Command(ThreadMessageCommand::NewConnect);
                    let _ = server_write_clone_0.send(server_message);
                    let mut buffer = vec![0; 65536];

                    // Uuid 확인. 성공, 실패시 메시지 보냄.
                    let (am_read_half, am_write_half, uuid) = match uuid_check(stream, &mut buffer).await {
                        Ok(((am_read_half, am_write_half), uuid)) => {
                            let mut write_half = am_write_half.lock().await;

                            println!("am tcp stream is ok!");

                            let tcp_message = TcpMessage {
                                send_type: "set_uuid".to_string(),
                                command: ErrorLevel::Ok.to_string(),
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
                                command: ErrorLevel::Fatal.to_string(),
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
                    thread_write_map_clone_0.insert(uuid, thread_write);
                    let mut in_tasks = JoinSet::new();

                    in_tasks.spawn(async move { ;
                        player_handle(am_read_half, am_write_half, uuid, main_thread_write_clone_0, thread_read, buffer, thread_write_map_clone_0).await
                    });

                    if let Some(res) = in_tasks.join_next().await {
                        match res {
                            Ok(result) => match result {
                                Ok(uuid) => {
                                    thread_write_map_clone_1.remove(&uuid);
                                    let _ = server_write_clone_1.send(ThreadMessage::Command(ThreadMessageCommand::EndOfConnect(uuid)));
                                }
                                Err((uuid, err)) => {
                                    thread_write_map_clone_1.remove(&uuid);
                                    eprintln!("err: {}", err);
                                }
                            }
                            Err(err) => {
                                eprintln!("task panic!: {}", err);
                                thread_write_map_clone_1.retain(|_, sender| !sender.is_closed());
                            }
                        }
                    }
                });
            },
            Err(err) => {
                eprintln!("tcp server err: {}", err);
            }
        }

        let Some(read) = server_read.recv().await else {
            continue
        };

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
