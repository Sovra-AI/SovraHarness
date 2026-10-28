//! Opt-in check against `WhatsApp`'s real servers: the embedded client completes the
//! Noise handshake and `WhatsApp` issues a genuine pairing QR code. Scanning needs a phone,
//! so the test stops once the code arrives.
//!
//! `cargo test -p sovra-whatsapp --test live -- --ignored --nocapture`

use std::sync::Arc;
use std::time::Duration;

use sovra_core::connections::{ChannelEvent, ChannelStartOptions, MessagingChannel};
use sovra_whatsapp::WhatsAppChannel;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to WhatsApp's servers over the internet"]
async fn whatsapp_servers_issue_a_real_pairing_qr_code() {
    let dir = std::env::temp_dir().join(format!("sovra-wa-live-{}", std::process::id()));
    let channel = WhatsAppChannel::new(&dir);
    let (tx, rx) = async_channel::unbounded::<ChannelEvent>();
    let session = channel
        .start(
            ChannelStartOptions::default(),
            Arc::new(move |event| {
                let _ = tx.try_send(event);
            }),
        )
        .await
        .expect("client starts");

    let mut codes = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while codes.len() < 2 {
        let event = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("WhatsApp issued a QR code within 90 seconds")
            .expect("link still open");
        match event {
            ChannelEvent::QrCode {
                payload,
                matrix,
                expires_in_secs,
            } => {
                eprintln!(
                    "QR #{}: {} chars, {}x{} modules, valid {}s",
                    codes.len() + 1,
                    payload.len(),
                    matrix.len(),
                    matrix.first().map_or(0, Vec::len),
                    expires_in_secs
                );
                assert!(payload.starts_with("2@"), "{payload}");
                assert_eq!(payload.matches(',').count(), 4, "{payload}");
                assert!(matrix.len() >= 45, "a real payload needs a large QR");
                assert!(matrix.iter().all(|row| row.len() == matrix.len()));
                assert!(expires_in_secs > 0);
                codes.push(payload);
            }
            ChannelEvent::Connecting => {}
            other => panic!("unexpected event before the QR code: {other:?}"),
        }
    }
    assert_ne!(codes[0], codes[1], "codes rotate");
    session.stop().await;
    channel.forget().expect("credentials removed");
    let _ = std::fs::remove_dir_all(&dir);
}
