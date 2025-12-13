use crate::{
    client::Client,
    dispatcher::{Dispatcher, DispatcherMessage},
};
use futures_util::{StreamExt, TryFutureExt};
use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
    sync::mpsc::{self},
};
use tokio_tungstenite::accept_async;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};
mod client;
mod dispatcher;
mod protocol;

const TRACING_LEVEL: tracing::Level = tracing::Level::INFO;
const DISPATCHER_CHANNEL_BUFFER_SIZE: usize = 100;
const CLIENT_CHANNEL_CAPACITY: usize = 32;
const IP: &str = "127.0.0.1";
const PORT: u16 = 8080;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt().with_max_level(TRACING_LEVEL).init();
    let ct = CancellationToken::new();

    tokio::spawn(shutdown_task(ct.clone()));

    let (disp_tx, disp_rx) = mpsc::channel(DISPATCHER_CHANNEL_BUFFER_SIZE);
    tokio::spawn(Dispatcher::new(disp_rx).run(ct.clone()));

    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::from_str(IP)?), PORT);
    let listener = TcpListener::bind(server_addr).await?;
    info!("Listening on {}:{}", IP, PORT);
    let conn_task_tracker = TaskTracker::new();

    loop {
        tokio::select! {
            biased;
            _ = ct.cancelled() => {
                info!("No longer accepting new connections");
                break;
            },
            res = listener.accept() => {
                match res {
                    Ok((tcp_stream, addr)) => {
                        conn_task_tracker.spawn(
                            tcp_handler(tcp_stream, disp_tx.clone(), addr).inspect_err(|e| error!("{}", e)));
                    },
                    Err(e) => error!("{}", e),
                }
            }
        }
    }

    conn_task_tracker.close();
    conn_task_tracker.wait().await;
    info!("All connections closed");

    Ok(())
}

async fn tcp_handler(
    tcp_stream: TcpStream,
    disp_tx: mpsc::Sender<DispatcherMessage>,
    addr: SocketAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let ws_stream = accept_async(tcp_stream).await?;
    let (ws_tx, ws_rx) = ws_stream.split();

    Client::new(addr, ws_tx, ws_rx, disp_tx).run().await?;

    Ok(())
}

async fn shutdown_task(ct: CancellationToken) {
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Shutting down...");
            ct.cancel();
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }
}
