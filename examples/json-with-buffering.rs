use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;

use futures::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

fn source_test_stream() -> impl Stream<Item = MyTestStructure> {
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter(vec![
        MyTestStructure {
            some_test_field: "test1".to_string()
        };
        100000
    ])
}

async fn test_json_array_stream() -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .buffering_ready_items(1000)
        .json_array(source_test_stream())
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/json-array-buffering", get(test_json_array_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
