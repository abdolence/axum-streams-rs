use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;
use futures::{stream, Stream, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

/// A stream slow enough that the progress reports have something to report.
fn source_test_stream() -> impl Stream<Item = MyTestStructure> {
    tokio_stream::StreamExt::throttle(
        stream::iter(vec![
            MyTestStructure {
                some_test_field: "test1".to_string()
            };
            1000
        ]),
        Duration::from_millis(10),
    )
    .boxed()
}

/// With the `tracing` feature enabled this reports the totals once at `INFO` when the response
/// ends, and with `RUST_LOG=axum_streams=debug` a progress line per second while it runs:
///
/// ```text
/// DEBUG axum_streams::stream_body{format="json_array"}: Streaming an HTTP body items=98 bytes=2842 elapsed_ms=1001
/// DEBUG axum_streams::stream_body{format="json_array"}: Streaming an HTTP body items=197 bytes=5713 elapsed_ms=2002
/// INFO  axum_streams::stream_body{format="json_array" items=1000 bytes=29002 elapsed_ms=10152 outcome="completed"}: Finished streaming an HTTP body ...
/// ```
///
/// The counters are also recorded as fields on the `axum_streams::stream_body` span, so a
/// collector reading span attributes sees them as structured values rather than log text.
async fn test_json_array_stream() -> impl IntoResponse {
    StreamBodyAs::json_array(source_test_stream())
}

/// Report progress more often than the default one second, and on item counts as well.
async fn test_json_array_stream_verbose() -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .progress_interval(Duration::from_millis(250))
        .progress_items(100)
        .json_array(source_test_stream())
}

/// The same accounting without tracing: `on_progress` is where you hook up your own metrics.
async fn test_json_array_stream_with_metrics(streamed_bytes: Arc<AtomicU64>) -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .on_progress(move |progress| {
            // Only the final snapshot carries a terminal outcome, so this counts each
            // response once. `Aborted` means the client went away mid-stream.
            if progress.outcome != StreamBodyOutcome::InProgress {
                streamed_bytes.fetch_add(progress.bytes, Ordering::Relaxed);
                println!(
                    "{} items / {} bytes in {:?} ({})",
                    progress.items,
                    progress.bytes,
                    progress.elapsed,
                    progress.outcome.as_str()
                );
            }
        })
        .json_array(source_test_stream())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // `axum_streams=info` reports one summary per response; `axum_streams=debug` adds the
    // progress reports, and `axum_streams=trace` every frame.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "axum_streams=debug".into()),
        )
        .init();

    let streamed_bytes = Arc::new(AtomicU64::new(0));

    let app = Router::new()
        .route("/json-array-stream", get(test_json_array_stream))
        .route(
            "/json-array-stream-verbose",
            get(test_json_array_stream_verbose),
        )
        .route(
            "/json-array-stream-with-metrics",
            get(move || test_json_array_stream_with_metrics(streamed_bytes.clone())),
        );

    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    axum::serve(listener, app).await?;

    Ok(())
}
