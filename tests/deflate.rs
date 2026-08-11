//! INTENT-HQ FORK: round-trip tests for the `deflate` feature.

#![cfg(all(feature = "deflate", feature = "connect", feature = "handshake"))]

use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpListener,
};
use tokio_tungstenite::{
    accept_async_with_config, connect_async_with_config,
    tungstenite::{
        extensions::{compression::deflate::DeflateConfig, Extensions},
        protocol::{Message, Role, WebSocketConfig},
    },
    WebSocketStream,
};

/// Wraps a stream and records every byte read from it, so tests can inspect
/// what actually went over the wire.
struct Sniffer<S> {
    inner: S,
    captured: Arc<Mutex<Vec<u8>>>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Sniffer<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &result {
            self.captured.lock().unwrap().extend_from_slice(&buf.filled()[before..]);
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Sniffer<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[allow(clippy::field_reassign_with_default)]
fn deflate_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.extensions.permessage_deflate = Some(DeflateConfig::default());
    config
}

/// Runs a deflate-configured server that captures the raw bytes of the first
/// message it receives, and a client that sends `payload` with `client_config`.
/// Returns (server response extensions header, received message, captured raw bytes).
async fn round_trip(
    client_config: Option<WebSocketConfig>,
    payload: String,
) -> (Option<String>, Message, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let captured = Arc::new(Mutex::new(Vec::new()));

    let server_captured = captured.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let stream = Sniffer { inner: stream, captured: server_captured.clone() };
        let mut ws =
            accept_async_with_config(stream, Some(deflate_config())).await.expect("handshake");
        // Discard the handshake request bytes: everything captured from here on
        // is WebSocket frame data.
        server_captured.lock().unwrap().clear();
        ws.next().await.expect("a message").expect("message read")
    });

    let (mut ws, response) =
        connect_async_with_config(format!("ws://{addr}"), client_config, false)
            .await
            .expect("connect");
    let negotiated = response
        .headers()
        .get("sec-websocket-extensions")
        .map(|v| v.to_str().expect("header is ascii").to_owned());
    ws.send(Message::text(payload)).await.expect("send");

    let message = server.await.expect("server task");
    let captured = captured.lock().unwrap().clone();
    (negotiated, message, captured)
}

#[tokio::test]
async fn deflate_round_trip_compresses_on_the_wire() {
    // Highly compressible payload.
    let payload = "intent-hq permessage-deflate round trip! ".repeat(2500);

    let (negotiated, message, captured) = round_trip(Some(deflate_config()), payload.clone()).await;

    let negotiated = negotiated.expect("server agreed on an extension");
    assert!(negotiated.contains("permessage-deflate"), "unexpected extension: {}", negotiated);
    assert_eq!(message.into_text().expect("text message").as_str(), payload);

    // RSV1 (0x40) set on the first frame byte marks a compressed message.
    assert_eq!(captured[0] & 0x40, 0x40, "RSV1 must be set on a compressed frame");
    assert_eq!(captured[0] & 0x0f, 0x1, "expected a text frame");
    // The on-the-wire bytes (frame headers included) must be much smaller than
    // the plaintext.
    assert!(
        captured.len() < payload.len() / 2,
        "expected compressed wire data, got {} bytes for {} bytes of plaintext",
        captured.len(),
        payload.len()
    );
}

#[tokio::test]
async fn client_without_deflate_stays_uncompressed() {
    let payload = "intent-hq permessage-deflate round trip! ".repeat(2500);

    let (negotiated, message, captured) = round_trip(None, payload.clone()).await;

    assert_eq!(negotiated, None, "no extension may be negotiated");
    assert_eq!(message.into_text().expect("text message").as_str(), payload);
    assert_eq!(captured[0] & 0x40, 0, "RSV1 must not be set");
    assert!(captured.len() > payload.len(), "expected uncompressed wire data");
}

// NOTE: constructing a *negotiated* `Extensions` (as intentd's manual server
// upgrade will) requires a public negotiation API on the tungstenite fork,
// which is not exposed yet; this exercises the constructor with the default
// (no-extension) state.
#[tokio::test]
async fn from_raw_socket_with_extensions_round_trip() {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let mut client = WebSocketStream::from_raw_socket_with_extensions(
        client_stream,
        Role::Client,
        None,
        Extensions::default(),
    )
    .await;
    let mut server = WebSocketStream::from_raw_socket_with_extensions(
        server_stream,
        Role::Server,
        None,
        Extensions::default(),
    )
    .await;

    client.send(Message::text("ping")).await.expect("client send");
    let received = server.next().await.expect("a message").expect("server read");
    assert_eq!(received.into_text().expect("text message").as_str(), "ping");

    server.send(Message::text("pong")).await.expect("server send");
    let received = client.next().await.expect("a message").expect("client read");
    assert_eq!(received.into_text().expect("text message").as_str(), "pong");
}
