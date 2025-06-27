use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver},
};
use server::*;

#[tokio::main]
async fn main() {
    let (client_write, client_read) = unbounded_channel();
    let (server_write, server_read) = unbounded_channel();
    tokio::join!(uuid_tcp_server(client_read, server_write), server_loop(server_read, client_write));
}
