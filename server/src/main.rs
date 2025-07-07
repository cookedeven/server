use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver},
};
use server::*;

#[tokio::main]
async fn main() {
    let (client_write, client_read) = unbounded_channel();
    let (server_write, server_read) = unbounded_channel();
    let handle1 = tokio::spawn(uuid_tcp_server(client_read, server_write));
    let handle2 = tokio::spawn(server_loop(server_read, client_write));
    
    handle1.await.expect("uuid_tcp_server failed");
    handle2.await.expect("server_loop failed");

}
