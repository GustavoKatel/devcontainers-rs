use std::net::TcpListener;
use tokio;

/// request an tcp port: based on this code: https://github.com/babariviere/port_scanner-rs/blob/master/src/lib.rs
/// changed to be async
pub async fn request_open_port() -> Option<u16> {
    tokio::task::spawn_blocking(move || match TcpListener::bind("0.0.0.0:0") {
        Ok(a) => match a.local_addr() {
            Ok(a) => Some(a.port()),
            Err(_) => None,
        },
        Err(_) => None,
    })
    .await
    .unwrap_or(None)
}
