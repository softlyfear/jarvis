use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::{accept_hdr_async_with_config, tungstenite::{Message, protocol::WebSocketConfig, handshake::server::{Request, Response, ErrorResponse}}};

use super::events::{IpcAction, IpcEvent};

pub const IPC_PORT: u16 = 9712;
pub const IPC_ADDR: &str = "127.0.0.1";

static BROADCAST_TX: OnceCell<broadcast::Sender<IpcEvent>> = OnceCell::new();
static ACTION_HANDLER: OnceCell<Arc<RwLock<Option<Box<dyn Fn(IpcAction) + Send + Sync>>>>> = OnceCell::new();

// Initialize the IPC broadcast channel
pub fn init() -> broadcast::Sender<IpcEvent> {
    if let Some(tx) = BROADCAST_TX.get() {
        return tx.clone();
    }

    let (tx, _) = broadcast::channel::<IpcEvent>(32);
    BROADCAST_TX.set(tx.clone()).ok();
    ACTION_HANDLER.set(Arc::new(RwLock::new(None))).ok();
    
    info!("IPC: Broadcast channel initialized");
    tx
}

// Send event to all connected clients
pub fn send(event: IpcEvent) {
    if let Some(tx) = BROADCAST_TX.get() {
        match tx.send(event.clone()) {
            Ok(n) => {
                // audio levels stream ~30/s, keep them out of the log
                if n > 0 && !matches!(event, IpcEvent::AudioLevel { .. }) {
                    debug!("IPC: Sent {:?} to {} client(s)", event, n);
                }
            }
            Err(_) => {
                // no receivers, that's fine
            }
        }
    }
}

// Register handler for incoming actions from GUI
pub fn set_action_handler<F>(handler: F)
where
    F: Fn(IpcAction) + Send + Sync + 'static,
{
    if let Some(h) = ACTION_HANDLER.get() {
        *h.write() = Some(Box::new(handler));
    }
}

fn handle_action(action: IpcAction) {
    info!("IPC: Received action {:?}", action);
    
    // handle ping internally
    if matches!(action, IpcAction::Ping) {
        send(IpcEvent::Pong);
        return;
    }
    
    // forward to registered handler
    if let Some(handler_lock) = ACTION_HANDLER.get() {
        let handler = handler_lock.read();
        if let Some(ref h) = *handler {
            h(action);
        }
    }
}

// Start the WebSocket server (blocking)
pub async fn start_server() {
    let addr = format!("{}:{}", IPC_ADDR, IPC_PORT);
    let socket_addr: SocketAddr = addr.parse().expect("Invalid IPC address");

    let listener = match TcpListener::bind(&socket_addr).await {
        Ok(l) => {
            info!("IPC: WebSocket server listening on ws://{}", addr);
            l
        }
        Err(e) => {
            error!("IPC: Failed to bind to {}: {}", addr, e);
            return;
        }
    };

    // notify that we're ready
    send(IpcEvent::Started);

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("IPC: Client connecting from {}", peer_addr);
        
        tokio::spawn(handle_client(stream, peer_addr));
    }
}

async fn accept_client(stream: TcpStream) -> Result<tokio_tungstenite::WebSocketStream<TcpStream>, String> {
    let config = WebSocketConfig::default().max_message_size(Some(64 * 1024)).max_frame_size(Some(64 * 1024));
    let callback = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let origin = request.headers().get("origin");
        if origin.is_some_and(|h| h.to_str().is_err()) || !crate::ipc_access::allowed_origin(origin.and_then(|h| h.to_str().ok())) {
            let mut denied = ErrorResponse::new(Some("GUI origin required".into()));
            *denied.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
            return Err(denied);
        }
        Ok(response)
    };
    let handshake = accept_hdr_async_with_config(stream, callback, Some(config));
    tokio::time::timeout(std::time::Duration::from_secs(10), handshake).await
        .map_err(|_| "handshake timed out".to_string())?
        .map_err(|e| e.to_string())
}

async fn handle_client(stream: TcpStream, peer_addr: SocketAddr) {
    let ws_stream = match accept_client(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("IPC: Handshake failed for {}: {}", peer_addr, e);
            return;
        }
    };
    info!("IPC: Client connected: {}", peer_addr);

    let Some(tx) = BROADCAST_TX.get() else { return; };
    let mut event_rx = tx.subscribe();
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    loop {
        tokio::select! {
            // forward events to client
            event_result = event_rx.recv() => {
                match event_result {
                    Ok(event) => {
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(e) => {
                                error!("IPC: Failed to serialize event: {}", e);
                                continue;
                            }
                        };

                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            info!("IPC: Client {} disconnected (send failed)", peer_addr);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("IPC: Client {} lagged {} events", peer_addr, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("IPC: Broadcast channel closed");
                        break;
                    }
                }
            }

            // receive messages from client
            msg_result = ws_rx.next() => {
                match msg_result {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<IpcAction>(&text) {
                            Ok(action) => handle_action(action),
                            Err(e) => {
                                warn!("IPC: Invalid action from {}: {}", peer_addr, e);
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if ws_tx.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("IPC: Client {} sent close frame", peer_addr);
                        break;
                    }
                    Some(Err(e)) => {
                        error!("IPC: Error receiving from {}: {}", peer_addr, e);
                        break;
                    }
                    None => {
                        info!("IPC: Client {} stream ended", peer_addr);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("IPC: Client disconnected: {}", peer_addr);
}

pub fn has_clients() -> bool {
    if let Some(tx) = BROADCAST_TX.get() {
        tx.receiver_count() > 0
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[tokio::test]
    async fn handshake_rejects_websites_and_accepts_gui_and_native_clients() {
        for origin in [None, Some("http://tauri.localhost"), Some("http://localhost:1420"), Some("https://example.com")] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { accept_client(listener.accept().await.unwrap().0).await });
            let mut request = url.into_client_request().unwrap();
            if let Some(origin) = origin { request.headers_mut().insert("origin", origin.parse().unwrap()); }
            let client = tokio_tungstenite::connect_async(request).await;
            let accepted = server.await.unwrap();
            assert_eq!(client.is_ok(), origin != Some("https://example.com"));
            assert_eq!(accepted.is_ok(), client.is_ok());
        }
    }

    #[tokio::test]
    async fn oversized_commands_are_rejected_before_action_deserialization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut ws = accept_client(listener.accept().await.unwrap().0).await.unwrap();
            assert!(matches!(ws.next().await, Some(Err(tokio_tungstenite::tungstenite::Error::Capacity(_)))));
        });
        let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        client.send(Message::Text("x".repeat(70 * 1024).into())).await.unwrap();
        server.await.unwrap();
    }
}
