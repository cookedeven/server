use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = TcpStream::connect("127.0.0.1:8080").await?;
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);

    // 사용자 입력을 받을 채널
    let (tx, mut rx) = mpsc::channel::<String>(10);

    // 읽기 태스크
    tokio::spawn(async move {
        let mut line = String::new();
        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            print!("\r[서버] {}\n> ", line.trim());
            io::stdout().flush().unwrap();
            line.clear();
        }
    });

    // 사용자 입력 태스크
    tokio::spawn({
        let tx = tx.clone();
        async move {
            let stdin = io::stdin();
            let mut input = String::new();

            loop {
                print!("> ");
                io::stdout().flush().unwrap();

                input.clear();
                stdin.read_line(&mut input).unwrap();
                let msg = input.trim().to_string();
                if msg.is_empty() {
                    continue;
                }
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        }
    });

    // 메시지 전송 태스크
    while let Some(msg) = rx.recv().await {
        writer.write_all(msg.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}
