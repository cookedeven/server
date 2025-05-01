use tokio::{
    net::{TcpStream, UdpSocket},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::OnceCell
};
use uuid::Uuid;
use lazy_static::lazy_static;
use libCode::{SERVER_IP};

lazy_static! {
    static ref CLIENT_UUID: Uuid = Uuid::nil();
    static ref CLIENT_ID: OnceCell<usize> = OnceCell::new();
}

struct PlayerStream {
    tcp_stream: TcpStream,
    uuid: Uuid,
    name: String,
    id: u32,
    session_id: u32,
}

async fn tcp_client() {
    match TcpStream::connect(SERVER_IP).await {
        Ok(mut stream) => {
            let msg = "Hello TCP Server!";
            let _ = stream.write_all(msg.as_bytes()).await;

            let mut buf = vec![0; 1024];
            if let Ok(n) = stream.read(&mut buf).await {
                println!("TCP 응답: {}", String::from_utf8_lossy(&buf[..n]));
            }
        }
        Err(e) => eprintln!("TCP 연결 실패: {}", e),
    }
}

async fn udp_client() {
    let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    socket.connect(SERVER_IP).await.unwrap();

    let msg = "Hello UDP Server!";
    let _ = socket.send(msg.as_bytes()).await;

    let mut buf = vec![0; 1024];
    if let Ok(n) = socket.recv(&mut buf).await {
        println!("UDP 응답: {}", String::from_utf8_lossy(&buf[..n]));
    }
}

pub async fn uuid_tcp_client() {
    let mut stream = TcpStream::connect(SERVER_IP).await.unwrap();

    stream.write(b"REQUEST_UUID.NEW").await.unwrap();
    let mut buffer = vec![0; 512];
    let size = stream.read(&mut buffer).await.unwrap();
    let uid = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
    println!("uid: {}", uid);

    let mut stream = TcpStream::connect(SERVER_IP).await.unwrap();
    let msg = format!("CONNECT_UUID.{}", uid);
    stream.write(msg.as_bytes()).await.unwrap();
    let size = stream.read(&mut buffer).await.unwrap();
    println!("Server connect: {}", String::from_utf8_lossy(&buffer[..size]));
}

async fn get_uuid(id: usize, stream: &mut TcpStream) -> Result<(), ()> {
    let mut buffer = vec![0; 512];

    let command: Vec<u8> = [1].to_vec();
    let byte_id = id.to_le_bytes().to_vec();

    let message_chain: Vec<_> = command.into_iter().chain(byte_id).collect();

    let _ = stream.write_all(&message_chain).await;
    match stream.read(&mut buffer).await {
        Ok(size) => {
            let message = &buffer[..size];
        }
        Err(err) => eprintln!("read error: {}", err)
    }
    todo!()
}

pub async fn async_tcp_client() {
    let mut stream = TcpStream::connect(SERVER_IP).await.unwrap();
    
}

pub async fn tcp() {
    let mut stream = TcpStream::connect(SERVER_IP).await.unwrap();
    let _ = stream.write_all(b"yay");
}

pub struct ServerHandle {
    pub stream: TcpStream,
}
/*
impl ServerHandle {
    #[no_mangle]
    pub extern "C" fn send_server(&mut self, message: *const c_char) -> c_int {
        let a = message.to_string();
        let _ = self.stream.write_all();
        
        0
    }
}
*/