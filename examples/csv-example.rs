use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;
use std::net::SocketAddr;

use futures::prelude::*;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

fn source_test_stream() -> BoxStream<'static, MyTestStructure> {
    // Simulating a stream with a plain vector and throttling to show how it works
    Box::pin(
        stream::iter(vec![
            MyTestStructure {
                some_test_field: "test1".to_string()
            };
            1000
        ])
        .throttle(std::time::Duration::from_millis(50)),
    )
}

async fn test_csv_stream() -> impl IntoResponse {
    StreamBodyWith::csv(source_test_stream())

    // Which is the same as:
    // StreamBodyWith::new(
    //     CsvStreamFormat::new(
    //         true, // with_header
    //         b',', // CSV delimiter
    //     ),
    //     source_test_stream(),
    // )
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/csv-stream", get(test_csv_stream));

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
