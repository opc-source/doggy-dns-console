use std::time::Duration;

use doggy_dns_console::datasource::mcp::{InstanceTransport, RmcpTransport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn bearer_token_is_sent_once_without_double_prefix() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 8192];
        let read = socket.read(&mut buffer).await.unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let _ = tx.send(request);
        let _ = socket
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
            )
            .await;
    });

    let transport = RmcpTransport::new(Some("it-token".to_string()), Duration::from_secs(2));
    let _ = transport
        .call(
            &format!("http://{addr}/mcp"),
            "get_status",
            serde_json::json!({}),
        )
        .await;

    let request = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("server received no request")
        .expect("request channel closed");
    let lower = request.to_ascii_lowercase();
    assert!(
        lower.contains("authorization: bearer it-token\r\n"),
        "request was:\n{request}"
    );
    assert!(!lower.contains("bearer bearer"), "request was:\n{request}");
}
