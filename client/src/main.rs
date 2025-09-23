use tokio::net::TcpStream;
use serde_json::{Map, json};
use lib_code::*;

#[tokio::main]
async fn main() {
    let stream = TcpStream::connect(SERVER_IP).await.unwrap();

    let (mut read, mut write) = stream.into_split();

    let message = TcpMessage {
        send_type: "uuid".to_string(),
        command: "get".to_string(),
        uuid: String::new(),
        send_data: Map::new(),
        request_data: Map::new()
    };

    println!("send: {:?}", message);

    send_tcp_message(&mut write, message).await;

    let mut buffer = vec![0; 65536];
    let tcp_message = read_data(&mut read, &mut buffer).await.unwrap();

    println!("read: {:?}", tcp_message);

    let uuid = tcp_message.uuid;

    let mut send_data = Map::new();
    send_data_setting!(send_data,
        [uuid.to_string(), ("name".to_string(), json!("test"))]
    );

    let message = TcpMessage {
        send_type: "name".to_string(),
        command: String::new(),
        uuid: uuid.clone(),
        send_data,
        request_data: Map::new()
    };

    println!("send: {:?}", message);

    send_tcp_message(&mut write, message).await;

    let tcp_message = read_data(&mut read, &mut buffer).await.unwrap();

    println!("read: {:?}", tcp_message);

    let name = tcp_message.send_data.get(&uuid).unwrap().get("name").unwrap().as_str().unwrap();

    println!("name: {}", name);

    let mut send_data = Map::new();
    send_data_setting!(send_data,
        [uuid.clone(), ("matching_2".to_string(), json!(true))]
    );

    let message = TcpMessage {
        send_type: "connect".to_string(),
        command: String::new(),
        uuid: uuid.clone(),
        send_data,
        request_data: Map::new()
    };

    println!("send: {:?}", message);

    send_tcp_message(&mut write, message).await;

    let mut send_data = Map::new();
    send_data_setting!(send_data,
        [uuid.clone(), ("matching".to_string(), json!(false))]
    );

    let tcp_message = read_data(&mut read, &mut buffer).await.unwrap();

    println!("read: {:?}", tcp_message);

    let names: Vec<_> = tcp_message.send_data.iter().map(|(key, value)| value.get("name").unwrap().as_str().unwrap()).collect();

    println!("name: {:?}", names);

    /*
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let message = TcpMessage {
        send_type: "connect".to_string(),
        command: String::new(),
        uuid: uuid.clone(),
        send_data,
        request_data: Map::new()
    };

    println!("send: {:?}", message);

    send_tcp_message(&mut write, message).await;
    */
}