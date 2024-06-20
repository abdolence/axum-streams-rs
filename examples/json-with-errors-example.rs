use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;
use futures::{stream, Stream, StreamExt};

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use axum_streams::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

struct MyError {
    message: String,
}

impl Into<axum::Error> for MyError {
    fn into(self) -> axum::Error {
        axum::Error::new(self.message)
    }
}

fn source_test_stream() -> impl Stream<Item = Result<MyTestStructure, MyError>> {
    // Simulating a stream with a plain vector and throttling to show how it works
    tokio_stream::StreamExt::throttle(
        stream::iter(vec![
            MyTestStructure {
                some_test_field: "test1".to_string()
            };
            10000
        ])
        .enumerate()
        .map(|(idx, item)| {
            if idx != 0 && idx % 10 == 0 {
                Err(MyError {
                    message: format!("Error at index {}", idx),
                })
            } else {
                Ok(item)
            }
        }),
        std::time::Duration::from_millis(500),
    )
}

async fn test_json_array_stream() -> impl IntoResponse {
    StreamBodyAs::json_array_with_errors(source_test_stream())
}

async fn test_json_nl_stream() -> impl IntoResponse {
    StreamBodyAs::json_nl_with_errors(source_test_stream())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt().with_target(false).init();

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/json-array-stream", get(test_json_array_stream))
        .route("/json-nl-stream", get(test_json_nl_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    axum::serve(listener, app).await?;

    Ok(())
}
