use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::{Json, Router};
use axum_streams::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

#[derive(Serialize)]
struct Summary {
    received: u64,
    failed: u64,
}

/// Receives an upload as a stream, so a body of any size is processed a record at a time and
/// never held in memory.
async fn ingest(mut items: JsonNlStreamFrom<MyTestStructure>) -> Json<Summary> {
    let mut received = 0;
    let mut failed = 0;

    while let Some(item) = items.next().await {
        match item {
            Ok(_item) => received += 1,
            // JSON Lines records are independently framed, so a bad line is not the end of the
            // upload and the loop continues with the next one.
            Err(err) => {
                eprintln!("skipping a bad record: {err}");
                failed += 1;
            }
        }
    }

    Json(Summary { received, failed })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/ingest", post(ingest))
        // A streaming upload is exactly what the default body limit exists to stop, so a route
        // that wants one has to say so. The extractor honours the limit when it is set.
        .layer(DefaultBodyLimit::disable());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Send one good record, one malformed, one good, to show both paths.
    let body = "{\"some_test_field\":\"one\"}\nnot json\n{\"some_test_field\":\"two\"}\n";

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/ingest"))
        .header("content-type", "application/jsonstream")
        .body(body)
        .send()
        .await?;

    println!("server replied: {}", response.text().await?);

    Ok(())
}
