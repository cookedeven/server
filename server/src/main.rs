use std::collections::HashMap;
use std::sync::Arc;
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    spawn
};
use uuid::Uuid;
use lazy_static::lazy_static;

type ShardState = Arc<Mutex<HashMap<String, String>>>;

const SERVER_IP: &'static str = "127.0.0.1:8080";

lazy_static!{
    static ref VEC_DB: Arc<Mutex<Vec<Uuid>>> = Arc::new(Mutex::new(Vec::new()));
}

lazy_static!{
    static ref HASHMAP_DB: Arc<Mutex<HashMap<String, Uuid>>> = Arc::new(Mutex::new(HashMap::new()));
}

lazy_static!{
    static ref UID_UPDATE: ShardState = Arc::new(Mutex::new(HashMap::new()));
}

async fn handle_tcp_connection(mut stream: TcpStream) {
    let mut buf = vec![0; 1024];
    match stream.read(&mut buf).await {
        Ok(n) if n > 0 => {
            let msg = String::from_utf8_lossy(&buf[..n]);
            println!("TCP 클라이언트 메시지: {}", msg);

            let response = format!("서버 응답 (TCP): {}", msg);
            let _ = stream.write_all(response.as_bytes()).await;
        }
        _ => println!("TCP 읽기 실패 또는 연결 종료"),
    }
}

async fn run_tcp_server() {
    let listener = TcpListener::bind(SERVER_IP).await.unwrap();
    println!("TCP 서버: {}", SERVER_IP);

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let uid = Uuid::new_v4().to_string();

        spawn(async move {
            println!("새로운 클라이언트 UID: {}", uid);
            let welcome = format!("WELCOME|UID:{}\n", uid);
            let _ = socket.write_all(welcome.as_bytes()).await;
            handle_tcp_connection(socket)
        });
    }
}

async fn run_udp_server() {
    let socket = UdpSocket::bind(SERVER_IP).await.unwrap();
    println!("UDP 서버: {}", SERVER_IP);

    let mut buf = vec![0; 1024];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, addr)) => {
                let msg = String::from_utf8_lossy(&buf[..n]);
                println!("UDP 메시지 from {}: {}", addr, msg);

                let response = format!("서버 응답 (UDP): {}", msg);
                let _ = socket.send_to(response.as_bytes(), addr).await;
            }
            Err(e) => eprintln!("UDP 수신 오류: {}", e),
        }
    }
}

fn get_id(text: &str) -> usize {
    let splits: Vec<&str> = text.splitn(2, ":").collect();
    splits[1].trim().parse().unwrap()
}

fn get_name(text: &str) -> String {
    String::new()
}

fn set_new_name(text: &str) -> String {
    String::new()
}

async fn uuid_tcp_handle(mut stream: TcpStream, state: ShardState) {
    let mut buffer = vec![0; 1024];
    if let Ok(size) = stream.read(&mut buffer).await {
        let message = String::from_utf8_lossy(&buffer[..size]);
        let trimmed = message.trim();

        if trimmed.starts_with("REQUEST_UUID") {
            let parts: Vec<&str> = trimmed.splitn(2, ".").collect();
            let mut state = 0;
            match parts[1] {
                "ID_NUM" => state = 1,
                "PLAYER_NAME" => state = 2,
                "NEW" => state = 3,
                _ => state = 4
            }
            match state {
                0 => eprintln!("not set state!"),
                1 => {
                    let id = get_id(&parts[1]);
                    if let Some(&uuid) = VEC_DB.lock().await.get(id) {
                        let _ = stream.write(uuid.to_string().as_bytes());
                    } else {
                        let _ = stream.write(b"invalid id");
                    }
                },
                2 => {
                    let name = get_name(&parts[1]);
                    if let Some(&uuid) = HASHMAP_DB.lock().await.get(&name) {
                        let _ = stream.write(uuid.to_string().as_bytes());
                    } else {
                        let _ = stream.write(b"invalid name");
                    }
                },
                3 => {
                    let new_name = set_new_name(&parts[1]);
                    let uuid = Uuid::new_v4();
                    VEC_DB.lock().await.push(uuid);
                    HASHMAP_DB.lock().await.insert(new_name, uuid);
                    let _ = stream.write(uuid.to_string().as_bytes()).await;
                }
                4 => eprintln!("invalid command! splits: {:?}", parts),
                _ => eprintln!("invalid state!"),
            }
        } else if trimmed.starts_with("CONNECT_UUID") {
            let parts: Vec<&str> = trimmed.splitn(2, ".").collect();
            let uuid = parts[1].trim();
            let mut state = state.lock().await;
            if state.contains_key(uuid) {
                state.insert(uuid.to_string(), "updated".into());
                let _ = stream.write(format!("connect to UID! name: {}", ).as_bytes());
            } else {
                let _ = stream.write(b"invalid UID");
            }
        } else {
            eprintln!("invalid command! message: {}", message);
        }
    }
}

async fn uuid_tcp_server() {
    let listener = TcpListener::bind(SERVER_IP).await.unwrap();
    let state: ShardState = Arc::new(Mutex::new(HashMap::new()));
    println!("Server listening on {}", SERVER_IP);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                spawn(async move {
                    uuid_tcp_handle(stream, state).await;
                });
            }
            Err(e) => {
                eprintln!("Connection failed: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() {
    tokio::join!(uuid_tcp_server());
}
