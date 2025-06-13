use server::uuid_tcp_server;

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver};
use tokio::task;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug)]
struct MessageToB {
    sender_id: Uuid,
    content: String,
}

#[derive(Debug)]
struct MessageFromB {
    content: String,
}

#[tokio::main]
async fn main() {
    // 중앙 수신 채널 (A -> B)
    let (tx_to_b, mut rx_at_b): (UnboundedSender<MessageToB>, UnboundedReceiver<MessageToB>) = unbounded_channel();

    // B → A 라우팅 테이블
    let routing_map: Arc<Mutex<HashMap<Uuid, UnboundedSender<MessageFromB>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // 중앙 처리 태스크 (B 역할)
    let routing_map_clone = Arc::clone(&routing_map);
    task::spawn(async move {
        while let Some(msg) = rx_at_b.recv().await {
            println!("[B] Received from {}: {}", msg.sender_id, msg.content);

            // 조건 예시: 전체에게 브로드캐스트
            if msg.content == "broadcast" {
                for (id, tx) in routing_map_clone.lock().unwrap().iter() {
                    let _ = tx.send(MessageFromB {
                        content: format!("Broadcast from B to {}", id),
                    });
                }
            } else {
                // 특정 발신자에게만 응답
                if let Some(tx) = routing_map_clone.lock().unwrap().get(&msg.sender_id) {
                    let _ = tx.send(MessageFromB {
                        content: format!("Direct reply to {}", msg.sender_id),
                    });
                }
            }
        }
    });

    // A₁, A₂, A₃ 스레드 생성
    for _ in 0..3 {
        let sender_id = Uuid::new_v4();
        let tx_to_b_clone = tx_to_b.clone();

        let (rx_from_b_tx, mut rx_from_b): (UnboundedSender<MessageFromB>, UnboundedReceiver<MessageFromB>) =
            unbounded_channel();

        routing_map.lock().unwrap().insert(sender_id, rx_from_b_tx);

        // 각 송신자 스레드
        task::spawn(async move {
            // B에게 메시지 전송
            let _ = tx_to_b_clone.send(MessageToB {
                sender_id,
                content: "hello".into(),
            });

            // B에게 브로드캐스트 요청
            let _ = tx_to_b_clone.send(MessageToB {
                sender_id,
                content: "broadcast".into(),
            });

            // B로부터 응답 수신
            while let Some(msg) = rx_from_b.recv().await {
                println!("[{}] Received from B: {:?}", sender_id, msg);
            }
        });
    }

    // 테스트 대기
    thread::time::sleep(tokio::time::Duration::from_secs(5)).await;
}

/*
#[tokio::main]
async fn main() {
    let (tx, mut rx): (UnboundedSender<String>, UnboundedReceiver<String>) = unbounded_channel();
    tokio::join!(uuid_tcp_server());
}
*/