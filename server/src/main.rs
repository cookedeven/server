use server::uuid_tcp_server;

#[tokio::main]
async fn main() {
    tokio::join!(uuid_tcp_server());
}