use std::net::SocketAddr;
use tokio::{
    net::{
        tcp::OwnedWriteHalf,
        TcpListener,
        TcpStream,
        UdpSocket
    },
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{broadcast, Mutex}

};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let udp = UdpSocket::bind("127.0.0.1:8080").await?;
    let (tx, _rx) = broadcast::channel(10);
    let clients = Arc::new(Mutex::new(Vec::new())); // `OwnedWriteHalf`를 직접 저장하지 않음

    println!("서버가 127.0.0.1:8080에서 실행 중...");

    let mut buf = [0; 1024];
    loop {
        let (socket, addr) = listener.accept().await?;
        println!("클라이언트 연결됨: {}", addr);

        let tx = tx.clone();
        let clients = Arc::clone(&clients);
        let udp_clients = Arc::clone(&clients);

        tokio::spawn(handle_client(socket, tx.clone(), tx.subscribe(), clients));
        tokio::spawn(handle_udp_client(udp_socket, tx, tx.subscribe(), udp_clients));
    }
}

async fn handle_client(socket: TcpStream, tx: broadcast::Sender<String>, mut rx: broadcast::Receiver<String>, clients: Arc<Mutex<Vec<OwnedWriteHalf>>>) -> anyhow::Result<()> {
    let (reader, writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    {
        let mut clients_lock = clients.lock().await;
        clients_lock.push(writer);
    }

    let clients = Arc::clone(&clients);

    let read_task = tokio::spawn(async move {
        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            let msg = line.trim().to_string();
            if !msg.is_empty() {
                tx.send(msg).unwrap();
            }
            line.clear();
        }
    });

    let write_task = tokio::spawn(async move {
        loop {
            let msg = match rx.recv().await {
                Ok(m) => m,
                Err(_) => break,
            };

            let mut clients_lock = clients.lock().await;
            let mut i = 0;

            while i < clients_lock.len() {
                let client = &mut clients_lock[i];

                if client.write_all(msg.as_bytes()).await.is_err()
                    || client.write_all(b"\n").await.is_err()
                {
                    clients_lock.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    });

    tokio::try_join!(read_task, write_task)?;

    Ok(())
}

async fn handle_udp_client(socket: SocketAddr, tx: broadcast::Sender<String>, mut rx: broadcast::Receiver<String>, clients: Arc<Mutex<Vec<OwnedWriteHalf>>>) -> anyhow::Result<()> {


    Ok(())
}