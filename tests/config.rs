//! Configuration forwarding on one stream; these are protocol-state tests, not TLS/auth tests.
use std::{
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    task::{Context, Waker},
};

use futures_util::{SinkExt, StreamExt};
use tokio::io::{duplex, AsyncWriteExt};
use tokio_tungstenite::{
    tungstenite::{
        protocol::{Role, WebSocketConfig},
        Error, Message,
    },
    WebSocketStream,
};

fn config(limit: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(8192)
        .write_buffer_size(0)
        .max_write_buffer_size(8448)
        .max_message_size(Some(limit))
        .max_frame_size(Some(limit))
}
fn frame(opcode: u8, final_frame: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![opcode | if final_frame { 128 } else { 0 }];
    if bytes.len() < 126 {
        out.push(bytes.len() as u8);
    } else if bytes.len() <= 65535 {
        out.push(126);
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(bytes);
    out
}

#[tokio::test]
async fn buffered_following_frame_survives_limit_transition() {
    let (io, _peer) = duplex(16);
    let mut bytes = frame(1, true, b"hello");
    bytes.extend(frame(1, true, &vec![b'x'; 65536]));
    let mut ws =
        WebSocketStream::from_partially_read(io, bytes, Role::Client, Some(config(8192))).await;
    assert_eq!(ws.next().await.unwrap().unwrap(), Message::text("hello"));
    let address = ws.get_ref() as *const _;
    ws.set_config(|c| {
        c.max_frame_size = Some(65536);
        c.max_message_size = Some(65536);
    });
    assert_eq!(ws.get_ref() as *const _, address);
    assert_eq!(ws.get_config().read_buffer_size, 8192);
    assert_eq!(ws.get_config().max_write_buffer_size, 8448);
    assert_eq!(ws.next().await.unwrap().unwrap().into_text().unwrap().len(), 65536);
}

#[tokio::test]
async fn original_and_updated_frame_limits_reject_plus_one() {
    for limit in [8192, 65536] {
        for extra in [0, 1] {
            let (io, _peer) = duplex(16);
            let mut ws = WebSocketStream::from_partially_read(
                io,
                frame(1, true, &vec![b'x'; limit + extra]),
                Role::Client,
                Some(config(8192)),
            )
            .await;
            if limit == 65536 {
                ws.set_config(|c| {
                    c.max_frame_size = Some(limit);
                    c.max_message_size = Some(limit);
                });
            }
            let result = ws.next().await.unwrap();
            if extra == 0 {
                assert_eq!(result.unwrap().into_text().unwrap().len(), limit);
            } else {
                assert!(matches!(result, Err(Error::Capacity(_))));
            }
        }
    }
}

#[tokio::test]
async fn fragmented_message_state_and_aggregate_limit_survive_update() {
    for extra in [0, 1] {
        let (io, mut peer) = duplex(131072);
        let mut ws = WebSocketStream::from_partially_read(
            io,
            frame(1, false, b"prefix"),
            Role::Client,
            Some(config(8192)),
        )
        .await;
        // Poll exactly once: consumes first fragment but cannot complete the message.
        let mut next = Box::pin(ws.next());
        assert!(next.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
        drop(next);
        ws.set_config(|c| {
            c.max_message_size = Some(65536);
            c.max_frame_size = Some(65536);
        });
        peer.write_all(&frame(0, true, &vec![b'x'; 65536 - 6 + extra])).await.unwrap();
        let result = ws.next().await.unwrap();
        if extra == 0 {
            let text = result.unwrap().into_text().unwrap();
            assert_eq!(text.len(), 65536);
            assert!(text.starts_with("prefix"));
        } else {
            assert!(matches!(result, Err(Error::Capacity(_))));
        }
    }
}

#[tokio::test]
async fn setter_keeps_pending_output_and_close_state() {
    let (io, peer) = duplex(256);
    let mut ws = WebSocketStream::from_raw_socket(
        io,
        Role::Server,
        Some(config(8192).write_buffer_size(4096)),
    )
    .await;
    let mut other = WebSocketStream::from_raw_socket(peer, Role::Client, None).await;
    ws.feed(Message::text("retained output")).await.unwrap();
    ws.set_config(|c| {
        c.max_frame_size = Some(65536);
        c.max_message_size = Some(65536);
    });
    ws.flush().await.unwrap();
    assert_eq!(other.next().await.unwrap().unwrap(), Message::text("retained output"));
    ws.close(None).await.unwrap();
    ws.set_config(|c| c.max_message_size = Some(8192));
    assert!(ws.send(Message::text("must not revive")).await.is_err());
}

#[tokio::test]
async fn invalid_threshold_panics_without_rollback_and_does_no_io() {
    let (io, _peer) = duplex(16);
    let mut ws = WebSocketStream::from_raw_socket(io, Role::Client, Some(config(8192))).await;
    assert!(catch_unwind(AssertUnwindSafe(
        || ws.set_config(|c| c.max_write_buffer_size = c.write_buffer_size)
    ))
    .is_err());
    // Matches the underlying setter; callers must not assume transactional repair.
    assert_eq!(ws.get_config().max_write_buffer_size, ws.get_config().write_buffer_size);
}
