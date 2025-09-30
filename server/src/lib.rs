use std::{
    str::FromStr,
    sync::Arc,
    collections::HashMap,
    net::SocketAddr
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
    task::{JoinSet, yield_now},
};
use uuid::Uuid;
use dashmap::DashMap;
use lazy_static::lazy_static;
use serde_json::{json, Map, Value};
use lib_code::{ErrorLevel, MatchingState, Error, SERVER_IP, TcpMessage, UserData, AD, AM, send_data_setting, request_data_setting, send_tcp_message, read_data, am, ad};

pub type Name = String;
pub type PlayerData = AD<Uuid, AD<String, Value>>;
pub type SessionID = i64;

macro_rules! end_of_tasks {
    ($arc_mutex_vec_unbounded_sender_thread_message:expr) => {
        $arc_mutex_vec_unbounded_sender_thread_message.lock().await.iter().for_each(|sender| {
            let _ = sender.send(ThreadMessage::Command(ThreadMessageCommand::EndOfTask));
        });
    };
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
    matching_state_2: MatchingState,
    matching_state_4: MatchingState,
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

pub enum MessageCommand {
    AddPlayer2(Uuid),
    RemovePlayer2(Uuid),
    AddPlayer4(Uuid),
    RemovePlayer4(Uuid),
    SendTcpMessage(TcpMessage),
}

fn name_get(player_data: &PlayerData, get_uuid: Uuid, uuid: Uuid, write_half: &mut OwnedWriteHalf) -> Result<String, TcpMessage> {
    player_data.get(&get_uuid).and_then(|data| data.get("name").map(|name_ref| name_ref.value().clone())).and_then(|name| {
        match name {
            Value::String(name) => Some(name),
            _ => None
        }
    }).ok_or_else(|| {
        println!("이름 가져오기 실패!");
        let mut send_data = Map::new();
        send_data_setting!(send_data,
            [uuid.to_string(), ("error".to_string(), json!(format!("invalid uuid: {}", get_uuid)))]
        );

        let message = TcpMessage {
            send_type: "error".to_string(),
            command: ErrorLevel::Warning.to_string(),
            uuid: uuid.to_string(),
            send_data,
            request_data: Map::new()
        };

        message
    })
}

async fn uuid_check(tcp_stream: TcpStream, buffer: &mut [u8]) -> Result<((AM<OwnedReadHalf>, AM<OwnedWriteHalf>), Uuid), (Error, OwnedReadHalf, OwnedWriteHalf)> {
    let (mut read_half, write_half) = tcp_stream.into_split();
    
    let tcp_message = match read_data(&mut read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err((err, read_half, write_half))
    };

    if &tcp_message.send_type != "uuid" {
        return Err((Error::NotFound, read_half, write_half))
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
                Err(_) => return Err((Error::InvalidUUID, read_half, write_half)),
            };
            let player_table = &PLAYER_TABLE;
            match player_table.get(&uuid) {
                Some(stream) => Ok((stream.clone(), uuid)),
                None => Err((Error::NotFound, read_half, write_half))
            }
        }
        _ => Err((Error::CommandNotFound, read_half, write_half))
    }
}

async fn name_check(read_half: &mut OwnedReadHalf, uuid: Uuid, buffer: &mut [u8]) -> Result<Name, Error> {
    let tcp_message = match read_data(read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return Err(err)
    };

    if tcp_message.send_type != "name" {
        return Err(Error::CommandNotFound);
    }

    let data_value = tcp_message.send_data.get(&uuid.to_string()).ok_or_else(|| Error::NotFound)?;
    let data = data_value.as_object().ok_or_else(|| Error::InvalidType)?;
    let name_value = data.get("name").ok_or_else(|| Error::NotFound)?;
    let name = name_value.as_str().ok_or_else(|| Error::InvalidType)?;

    Ok(name.to_string())
}

fn send_data_handle(send_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Vec<Result<MessageCommand, Error>> {
    if send_data.is_empty() {
        return Vec::new()
    }

    let mut result = Vec::new();

    println!("send_data: {:?}", send_data);

    for (send_data_uuid, data_value) in send_data {
        println!("send_data_uuid: {:?}", send_data_uuid);

        let data = match data_value.as_object() {
            Some(data) => data,
            None => {
                result.push(Err(Error::InvalidType));
                continue
            }
        };

        if send_data_uuid == uuid.to_string() {
            for (data_name, value) in data {
                println!("data_name: {:?}", data_name);
                match data_name.as_str() {
                    "matching_2" => {
                        println!("value: {:?}", value);
                        let match_state = match value.as_bool() {
                            Some(match_state) => match_state,
                            None => {
                                result.push(Err(Error::InvalidType));
                                continue
                            }
                        };

                        if match_state {
                            if state.matching_state_2 == MatchingState::Nothing || state.matching_state_2 == MatchingState::None {
                                println!("매칭중!: {}", name);
                                state.matching_state_2 = MatchingState::Matching;
                                result.push(Ok(MessageCommand::AddPlayer2(uuid.clone())));
                            }
                        } else {
                            if state.matching_state_2 == MatchingState::Matching {
                                println!("매칭 종료!: {}", name);
                                state.matching_state_2 = MatchingState::Nothing;
                            }
                        }
                    }
                    _ => continue
                }
            }
        }
    }

    result
}

fn request_data_handle(request_data: UserData, uuid: &Uuid, name: &Name, state: &mut UserState) -> Vec<Result<MessageCommand, Error> >{
    if request_data.is_empty() {
        return Vec::new()
    }

    let mut result = Vec::new();

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

                result.push(Ok(MessageCommand::SendTcpMessage(message)));
            }
            _ => result.push(Err(Error::InvalidType))
        }
    }
    
    result
}

async fn message_handle(am_read_half: AM<OwnedReadHalf>, am_write_half: AM<OwnedWriteHalf>, uuid: Uuid, name: &Name, am_state: AM<UserState>, buffer: &mut [u8], user_sender: AD<Uuid, UnboundedSender<ThreadMessage>>) -> (Result<(), Error>, Result<(), Vec<Error>>) {
    let mut read_half = am_read_half.lock().await;
    let tcp_message = match read_data(&mut read_half, buffer).await {
        Ok(tcp_message) => tcp_message,
        Err(err) => return (Err(err), Err(Vec::new()))
    };
    drop(read_half);

    if tcp_message.send_type.as_str() != "connect" {
        return (Err(Error::CommandNotFound), Err(Vec::new()));
    }

    let mut state = am_state.lock().await;

    let send_data_result = send_data_handle(tcp_message.send_data, &uuid, name, &mut state);
    let request_data_result = request_data_handle(tcp_message.request_data, &uuid, name, &mut state);

    drop(state);

    let (ok_data_result, err_data_result): (Vec<_>, Vec<_>) = send_data_result.into_iter().chain(request_data_result.into_iter()).partition(|result| result.is_ok());

    let ok_data_result: Vec<_> = ok_data_result.into_iter().filter_map(Result::ok).collect();
    let err_data_result: Vec<_> = err_data_result.into_iter().filter_map(Result::err).collect();

    let (first_result, second_result);

    if ok_data_result.is_empty() {
        first_result = Ok(());
    } else {
        for ok_result in ok_data_result {
            match ok_result {
                MessageCommand::AddPlayer2(uuid) => {
                    let mut match_queue_2 = MATCH_QUEUE_2.lock().await;
                    match_queue_2.push(uuid);
                    println!("매칭 진입! 인원수 2");
                }
                MessageCommand::RemovePlayer2(uuid) => {
                    let mut match_queue_2 = MATCH_QUEUE_2.lock().await;
                    match_queue_2.retain(|queue_uuid| *queue_uuid != uuid);
                    println!("매칭 종료! 인원수 2")
                }
                MessageCommand::AddPlayer4(uuid) => {
                    let mut match_queue_4 = MATCH_QUEUE_4.lock().await;
                    match_queue_4.push(uuid);
                }
                MessageCommand::RemovePlayer4(uuid) => {
                    let mut match_queue_4 = MATCH_QUEUE_4.lock().await;
                    match_queue_4.retain(|queue_uuid| *queue_uuid != uuid);
                }
                MessageCommand::SendTcpMessage(message) => {
                    let mut write_half = am_write_half.lock().await;
                    send_tcp_message(&mut write_half, message).await;
                }
            }
        }
        first_result = Ok(());
    }

    if err_data_result.is_empty() {
        second_result = Ok(());
    } else {
        second_result = Err(err_data_result);
    }




    (first_result, second_result)
}

async fn player_handle(am_read_half: AM<OwnedReadHalf>, am_write_half: AM<OwnedWriteHalf>, uuid: Uuid, name: Name, main_write: UnboundedSender<ThreadMessage>, thread_read: UnboundedReceiver<ThreadMessage>, mut buffer: Vec<u8>, user_senders: AD<Uuid, UnboundedSender<ThreadMessage>>) -> Result<Uuid, (Uuid, Error)> {
    let write_half_clone_0 = am_write_half.clone();
    let write_half_clone_1 = am_write_half.clone();
    let read_half_clone_0 = am_read_half.clone();
    let read_half_clone_1 = am_read_half.clone();
    let am_state = am!(UserState::default());
    let state_clone_0 = am_state.clone();
    let state_clone_1 = am_state.clone();
    let mut tasks = JoinSet::new();
    let am_tasks_sender = am!(Vec::new());
    let tasks_sender_clone = am_tasks_sender.clone();
    let (sender, mut receiver) = unbounded_channel();
    let tcp_thread_senders = USER_UNBOUNDED_SENDERS.clone();
    tcp_thread_senders.insert(uuid, sender.clone());

    tasks.spawn(async move {
        println!("thread spawned");
        let mut tasks_sender = tasks_sender_clone.lock().await;
        tasks_sender.push(sender);
        drop(tasks_sender);
        loop {
            let write_half = write_half_clone_0.clone();
            let read_half = read_half_clone_0.clone();
            let state = state_clone_0.clone();
            println!("읽기 준비중. uuid: {}", uuid);
            let user_sender = user_senders.clone();
            let (send_data_result, request_data_result) = message_handle(read_half, write_half, uuid, &name, state, &mut buffer, user_sender).await;

            if let Err(error) = send_data_result {
               if error.unrecoverable_error() {
                   println!("읽기 종료!");
                   return Err(Error::OtherError(error.into()));
               }
            }

            if let Err(error) = request_data_result {
               for err in error {
                   if err.unrecoverable_error() {
                       println!("읽기 종료!");
                       return Err(Error::OtherError(err.into()));
                   }
               }
            }

            yield_now().await;
        }
    });

    tasks.spawn(async move {
        loop {
            println!("플레이어 헨들 읽기 준비! uuid: {}", uuid);
            let Some(message) = receiver.recv().await else {
                return Err(Error::ConnectionClosed)
            };

            println!("플레이어 헨들에서 메시지 읽음! uuid: {}", uuid);

            match message {
                ThreadMessage::Data(ThreadMessageData::SessionID(session_id)) => {
                    let state = state_clone_1.lock().await;
                    if state.matching_state_2 == MatchingState::Matching {
                        println!("매칭");
                        state.matching_state_2 == MatchingState::Playing;
                        let session_senders = SERVER_UNBOUNDED_SENDERS.clone();
                        let Some(session_sender) = session_senders.get(&session_id) else {
                            return Err(Error::NotFound)
                        };

                    } else {
                        println!("매칭칭");
                    }
                }
                ThreadMessage::Data(_) => {}
                ThreadMessage::Command(ThreadMessageCommand::EndOfTask) => return Ok(()),
                ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)) => {
                    let mut write_half = write_half_clone_1.lock().await;
                    println!("매칭 완료! 인원수: 2 uuid: {}", uuid);
                    let player_data = USER_DATA.clone();
                    let name_1 = match name_get(&player_data, uuid_1, uuid, &mut write_half) {
                        Ok(name_1) => name_1,
                        Err(message) => {
                            println!("이름 불러오기 문제 발생!!!!!");
                            send_tcp_message(&mut write_half, message).await;
                            "ERRORRRRRRRRRRRRRRRRRRRRRRRRRRRR".to_string()
                        }
                    };

                    let name_2 = match name_get(&player_data, uuid_2, uuid, &mut write_half) {
                        Ok(name_2) => name_2,
                        Err(message) => {
                            println!("이름 불러오기 문제 발생!!!!!");
                            send_tcp_message(&mut write_half, message).await;
                            "ERRORRRRRRRRRRRRRRRRRRRRRRRRRRRR".to_string()
                        }
                    };

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

            yield_now().await;
        }
    });

    if let Some(result) = tasks.join_next().await {
        match result {
            Ok(task_result) => {
                match task_result {
                    Ok(()) => {
                        end_of_tasks!(am_tasks_sender);
                        Ok(uuid)
                    },
                    Err(error) => {
                        end_of_tasks!(am_tasks_sender);
                        Err((uuid, error))
                    },
                }
            },
            Err(err) => {
                end_of_tasks!(am_tasks_sender);
                Err((uuid, Error::OtherError(err.into())))
            }
        }
    } else {
        end_of_tasks!(am_tasks_sender);
        Err((uuid, Error::UndefinedError))
    }
}

async fn player_process(stream: TcpStream, addr: SocketAddr, server_write: UnboundedSender<ThreadMessage>, thread_write_map: AD<Uuid, UnboundedSender<ThreadMessage>>, main_thread_write: UnboundedSender<ThreadMessage>) {
    println!("new client!!!!!!! addr: {}", addr);
    let server_message = ThreadMessage::Command(ThreadMessageCommand::NewConnect);
    let _ = server_write.send(server_message);
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

    let _ = server_write.send(ThreadMessage::Command(ThreadMessageCommand::NewPlayer(uuid)));

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

    let user_data = USER_DATA.clone();
    let dash_map_name = HashMap::from([
        ("name".to_string(), json!(name)),
    ]).into_iter().collect();

    user_data.insert(uuid, Arc::new(dash_map_name));

    // 스레드 채널 생성
    let (thread_write, thread_read) = unbounded_channel();
    // 스레드 채널 추가
    thread_write_map.insert(uuid, thread_write);
    let thread_write_map_clone = thread_write_map.clone();
    let tasks = spawn(async move {
        player_handle(am_read_half, am_write_half, uuid, name, main_thread_write, thread_read, buffer, thread_write_map_clone).await
    });

    println!("task 종료!");
    match tasks.await {
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

async fn user_manager(mut thread_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>) {
    
}

async fn session_manager_2(session_id: SessionID, session_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>, uuid_1: Uuid, uuid_2: Uuid) -> Result<(), Error> {
    println!("세션 생성! id: {}, player_1_uuid: {}, player_2_uuid: {}", session_id, uuid_1, uuid_2);

    let user_senders = USER_UNBOUNDED_SENDERS.clone();
    let Some(user_1_sender) = user_senders.get(&uuid_1) else {
        return Err(Error::NotFound);
    };

    let Some(user_2_sender) = user_senders.get(&uuid_2) else {
        return Err(Error::NotFound);
    };

    println!("유저 핸들에 메시지 보냄!");
    let _ = user_1_sender.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)));
    let _ = user_2_sender.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_2, uuid_1)));

    let _ = user_1_sender.send(ThreadMessage::Data(ThreadMessageData::SessionID(session_id)));
    let _ = user_2_sender.send(ThreadMessage::Data(ThreadMessageData::SessionID(session_id)));

    Ok(())
}

async fn session_manager_4(session_id: SessionID, session_read: UnboundedReceiver<ThreadMessage>, main_write: UnboundedSender<ThreadMessage>, thread_senders: Arc<DashMap<Uuid, UnboundedSender<ThreadMessage>>>, uuid_1: Uuid, uuid_2: Uuid, uuid_3: Uuid, uuid_4: Uuid) {
    
}

pub async fn player_tcp_handle(mut server_read: UnboundedReceiver<ThreadMessage>, server_write: UnboundedSender<ThreadMessage>) -> Result<(), Error> {
    let server_addr = SocketAddr::from_str(SERVER_IP)?;
    let listener = TcpListener::bind(server_addr).await?;
    println!("Server listening on {}", SERVER_IP);

    let (main_thread_write, mut main_thread_read) = unbounded_channel();
    let thread_write_map = ad!();
    let mut tasks = JoinSet::new();

    let server_write_clone = server_write.clone();

    // /*
    tasks.spawn(async move {
        loop {
            let mut match_queue_2 = MATCH_QUEUE_2.lock().await;
            let mut match_queue_4 = MATCH_QUEUE_4.lock().await;
            if match_queue_2.len() >= 2 {
                println!("매칭 수락! 2");
                let uuid_1 = match_queue_2.remove(0);
                let uuid_2 = match_queue_2.remove(0);
                let _ = server_write_clone.send(ThreadMessage::Command(ThreadMessageCommand::NewSession2(uuid_1, uuid_2)));
            }

            if match_queue_4.len() >= 4 {
                println!("매칭 수락! 4");
                let uuid_1 = match_queue_4.remove(0);
                let uuid_2 = match_queue_4.remove(0);
                let uuid_3 = match_queue_4.remove(0);
                let uuid_4 = match_queue_4.remove(0);
                let _ = server_write_clone.send(ThreadMessage::Command(ThreadMessageCommand::NewSession4(uuid_1, uuid_2, uuid_3, uuid_4)));
            }

            yield_now().await;
        }
    });
    // */

    tasks.spawn(async move {
        loop {
            let Some(read) = server_read.recv().await else {
                continue
            };

            yield_now().await;
        }
    });

    loop {
        let server_write_clone = server_write.clone();
        let main_thread_write_clone = main_thread_write.clone();
        let thread_write_map_clone = thread_write_map.clone();

        match listener.accept().await {
            Ok((stream, addr)) => {
                println!("New connection from {}", addr);
                spawn(async move {
                    eprintln!("new connect task spawn");
                    player_process(stream, addr, server_write_clone, thread_write_map_clone, main_thread_write_clone).await;
                });
            }
            Err(err) => {
                eprintln!("tcp server err: {}", err);
            }
        };

        yield_now().await;
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

        println!("유저 스래드에서 메시지 읽음!");

        match read {
            ThreadMessage::Data(data) => {}
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

        yield_now().await;
    }
}
