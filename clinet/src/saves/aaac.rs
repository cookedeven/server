use std::io::{self, Write, Read};
use std::net::TcpStream;

fn main() -> io::Result<()> {
    // 서버에 연결
    let mut stream = TcpStream::connect("127.0.0.1:8080")?;
    println!("서버에 연결되었습니다.");

    // 보낼 메시지
    let message = "안녕하세요, 서버님!";
    stream.write_all(message.as_bytes())?;
    println!("메시지를 보냈습니다: {}", message);

    // 서버로부터 응답 받기
    let mut buffer = [0; 512];
    let n = stream.read(&mut buffer)?;
    let response = String::from_utf8_lossy(&buffer[..n]);
    println!("서버 응답: {}", response);

    Ok(())
}
