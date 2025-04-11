use std::fmt::format;
use tokio::net::{TcpStream, UdpSocket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::{Uuid, uuid};

const SERVER_IP: &'static str = "127.0.0.1:8080";

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

async fn uuid_tcp_client() {
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

#[tokio::main]
async fn main() {
    tokio::join!(uuid_tcp_client());
}
