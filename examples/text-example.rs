use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;

use futures::prelude::*;
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

use axum_streams::*;

fn source_test_stream() -> impl Stream<Item = String> {
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter(vec![
        "苟利国家生死以，岂因祸福避趋之？".to_string();
        1000
    ])
    .throttle(std::time::Duration::from_millis(50))
}

async fn test_text_stream() -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .content_type(HttpHeaderValue::from_static("text/plain; charset=utf-8"))
        .text(source_test_stream());
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/text-stream", get(test_text_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
