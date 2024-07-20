use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;

use futures::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field1: String,
    some_test_field2: String,
}

fn source_test_stream() -> impl Stream<Item = MyTestStructure> {
    use tokio_stream::StreamExt;
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter(vec![
        MyTestStructure {
            some_test_field1: "test1".to_string(),
            some_test_field2: "test2".to_string()
        };
        1000
    ])
    .throttle(std::time::Duration::from_millis(500))
}

async fn test_csv_stream() -> impl IntoResponse {
    StreamBodyAs::csv(source_test_stream())

    // If you need more control for CSV format, you can use the following code instead of the previous line:
    // StreamBodyAs::new(
    //     CsvStreamFormat::new(
    //         true, // with_header
    //         b',', // CSV delimiter
    //     ),
    //     source_test_stream().map(Ok::<_, axum::Error>),
    // )
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/csv-stream", get(test_csv_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
