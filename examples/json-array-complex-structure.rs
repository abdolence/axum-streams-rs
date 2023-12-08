use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;

use futures::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyItem {
    some_test_field: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyEnvelopeStructure {
    other_field: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    my_array: Vec<MyItem>,
}

fn source_test_stream() -> impl Stream<Item = MyItem> {
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter(vec![
        MyItem {
            some_test_field: "test1".to_string()
        };
        100
    ])
    .throttle(std::time::Duration::from_millis(50))
}

async fn test_json_array_stream() -> impl IntoResponse {
    let envelope = MyEnvelopeStructure {
        other_field: "test".to_string(),
        my_array: Vec::new(),
    };
    StreamBodyAs::json_array_with_envelope(source_test_stream(), envelope, "my_array")
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/json-array-stream", get(test_json_array_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
