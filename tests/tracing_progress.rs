//! Coverage for what the `tracing` feature actually emits.
//!
//! This lives in its own test binary on purpose. `tracing` caches each callsite's interest
//! globally, and a callsite first evaluated while no subscriber is registered is cached as
//! "never interested". Sharing a process with tests that install no subscriber therefore makes
//! a thread-local `set_default` race with that cache; a dedicated binary has no such neighbours.
#![cfg(all(feature = "tracing", feature = "text"))]

use axum::response::IntoResponse;
use futures::StreamExt;
use std::io;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

use axum_streams::*;

/// Installing a subscriber rebuilds `tracing`'s global callsite interest cache, so two tests
/// doing it at once would race exactly the way this binary exists to avoid. One at a time.
///
/// A `tokio` mutex rather than a `std` one: it is held across awaits, and it does not carry
/// poisoning, so one failing test cannot cascade into the others.
static TRACING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Drains a body of three three-byte items with a subscriber installed, and returns everything
/// that was logged.
async fn capture_at(level: tracing::Level, abort_after_first_frame: bool) -> String {
    let _serialized = TRACING.lock().await;
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(SharedWriter(buffer.clone()))
        .finish();

    // Not `set_global_default`: `capture_at` is called by several tests in this binary.
    let _guard = tracing::subscriber::set_default(subscriber);

    let items = futures::stream::iter((0..3).map(|index| format!("{index:03}")));
    let mut stream = StreamBodyAs::text(items)
        .into_response()
        .into_body()
        .into_data_stream();

    if abort_after_first_frame {
        let _ = stream.next().await;
        drop(stream);
    } else {
        while let Some(frame) = stream.next().await {
            let _ = frame;
        }
    }

    let captured = buffer.lock().unwrap().clone();
    String::from_utf8(captured).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn reports_the_summary_at_info() {
    let captured = capture_at(tracing::Level::INFO, false).await;

    assert!(captured.contains("INFO"), "unexpected output: {captured}");
    assert!(
        captured.contains("items=3"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("bytes=9"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains(r#"outcome="completed""#),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("http_streams_core::stream"),
        "the summary must be recorded on the body span: {captured}"
    );
    assert!(
        captured.contains(r#"format="text""#),
        "the span must name the format: {captured}"
    );
}

/// A body whose source stream fails partway, drained under a subscriber at `level`.
async fn capture_failure_at(level: tracing::Level) -> String {
    let _serialized = TRACING.lock().await;
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(SharedWriter(buffer.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let items = futures::stream::iter(vec![
        Ok("000".to_string()),
        Err(axum::Error::new("boom")),
        Ok("002".to_string()),
    ]);
    let mut stream = StreamBodyAs::text_with_errors(items)
        .into_response()
        .into_body()
        .into_data_stream();
    while let Some(frame) = stream.next().await {
        if frame.is_err() {
            break;
        }
    }
    drop(stream);

    let captured = buffer.lock().unwrap().clone();
    String::from_utf8(captured).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn reports_the_failure_summary_when_only_error_is_enabled() {
    // The gate must sit at the least verbose level the accounting emits at. Gating at INFO
    // meant `RUST_LOG=http_streams_core=error` built no instrumentation at all, so the totals of
    // the very responses that filter asks about were lost.
    let captured = capture_failure_at(tracing::Level::ERROR).await;

    assert!(
        captured.contains(r#"outcome="failed""#),
        "the failure summary must survive an ERROR-only filter: {captured}"
    );
    assert!(
        captured.contains("items=1"),
        "unexpected output: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn stays_quiet_below_info() {
    let captured = capture_at(tracing::Level::WARN, false).await;

    assert!(
        captured.is_empty(),
        "a successful body must report nothing above INFO: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_a_client_that_hung_up() {
    let captured = capture_at(tracing::Level::INFO, true).await;

    assert!(
        captured.contains(r#"outcome="aborted""#),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("items=1"),
        "unexpected output: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_progress_only_at_debug() {
    let info = capture_at(tracing::Level::INFO, false).await;
    assert!(
        !info.contains("Streaming an HTTP body"),
        "progress belongs to DEBUG, not INFO: {info}"
    );

    // `progress_interval(ZERO)` reports on every frame, so DEBUG must show progress lines
    // that INFO does not.
    let _serialized = TRACING.lock().await;
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(SharedWriter(buffer.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let items = futures::stream::iter((0..3).map(|index| format!("{index:03}")));
    let mut stream = StreamBodyAsOptions::new()
        .progress_interval(std::time::Duration::ZERO)
        .text(items)
        .into_response()
        .into_body()
        .into_data_stream();
    while let Some(frame) = stream.next().await {
        let _ = frame;
    }

    let captured = buffer.lock().unwrap().clone();
    let captured = String::from_utf8(captured).unwrap();
    assert!(
        captured.contains("Streaming an HTTP body"),
        "unexpected output: {captured}"
    );
}
