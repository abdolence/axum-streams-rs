use arrow::array::*;
use arrow::datatypes::*;
use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;
use std::sync::Arc;

use futures::prelude::*;
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

use axum_streams::*;

fn source_test_stream(schema: Arc<Schema>) -> impl Stream<Item = RecordBatch> {
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter((0i64..10i64).map(move |idx| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![idx, idx * 2, idx * 3])),
                Arc::new(StringArray::from(vec!["New York", "London", "Gothenburg"])),
                Arc::new(Float64Array::from(vec![40.7128, 51.5074, 57.7089])),
                Arc::new(Float64Array::from(vec![-74.0060, -0.1278, 11.9746])),
            ],
        )
        .unwrap()
    }))
    .throttle(std::time::Duration::from_millis(50))
}

async fn test_text_stream() -> impl IntoResponse {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("city", DataType::Utf8, false),
        Field::new("lat", DataType::Float64, false),
        Field::new("lng", DataType::Float64, false),
    ]));
    StreamBodyAs::arrow_ipc(Some(schema.clone()), source_test_stream(schema.clone()))
}

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/arrow-stream", get(test_text_stream));

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
