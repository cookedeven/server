use std::net::UdpSocket;

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind("127.0.0.1:0")?; // 임의의 포트 사용
    socket.connect("127.0.0.1:8080")?;

    let msg = b"Hello, UDP!";
    socket.send(msg)?;
    println!("메시지 전송 완료");

    let mut buf = [0; 1024];
    let size = socket.recv(&mut buf)?;
    let utf8 = String::from_utf8_lossy(&buf[..size]).to_string();
    println!("서버 응답: {}", utf8);

    Ok(())
}
