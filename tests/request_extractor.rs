#![cfg(all(
    feature = "json",
    feature = "csv",
    feature = "protobuf",
    feature = "arrow"
))]

//! Tests for the streamed request-body extractor.
//!
//! The primary check is a round trip against this crate's own encoder: encode a known vector
//! with `StreamBodyAs`, feed those bytes to the extractor, and assert the items come back.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, Request};
use axum::response::IntoResponse;
use axum_streams::*;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: u32,
    name: String,
}

fn items() -> Vec<Item> {
    vec![
        Item {
            id: 1,
            name: "one".into(),
        },
        Item {
            id: 2,
            name: "two".into(),
        },
    ]
}

/// Renders what this crate's encoder produces, so the two directions are tested against
/// each other rather than against a hand-written fixture that could drift.
async fn encoded(body: StreamBodyAs<'static>) -> Vec<u8> {
    let mut stream = body.into_response().into_body().into_data_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    out
}

/// Builds a request by hand, so framing can be exercised at chosen chunk boundaries without a
/// socket in the way.
fn request(content_type: &str, chunks: Vec<Vec<u8>>) -> Request {
    let stream = futures::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>));
    Request::builder()
        .method("POST")
        .uri("/ingest")
        .header("content-type", content_type)
        .body(Body::from_stream(stream))
        .unwrap()
}

fn split(bytes: &[u8], size: usize) -> Vec<Vec<u8>> {
    bytes.chunks(size).map(|c| c.to_vec()).collect()
}

#[tokio::test]
async fn json_nl_round_trips_from_this_crates_encoder() {
    let bytes = encoded(StreamBodyAs::json_nl(futures::stream::iter(items()))).await;

    let extracted = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(
        request("application/jsonstream", vec![bytes]),
        &(),
    )
    .await
    .expect("extraction must succeed");

    let got: Vec<Item> = extracted
        .map(|r| r.expect("no item may fail"))
        .collect()
        .await;
    assert_eq!(got, items());
}

#[tokio::test]
async fn json_array_round_trips_from_this_crates_encoder() {
    let bytes = encoded(StreamBodyAs::json_array(futures::stream::iter(items()))).await;

    let extracted = <JsonArrayStreamFrom<Item> as FromRequest<()>>::from_request(
        request("application/json", vec![bytes]),
        &(),
    )
    .await
    .expect("extraction must succeed");

    let got: Vec<Item> = extracted
        .map(|r| r.expect("no item may fail"))
        .collect()
        .await;
    assert_eq!(got, items());
}

/// The test that actually catches framing bugs: every possible split point.
#[tokio::test]
async fn json_nl_survives_every_chunk_boundary() {
    let bytes = encoded(StreamBodyAs::json_nl(futures::stream::iter(items()))).await;

    for size in 1..=bytes.len() {
        let extracted = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(
            request("application/jsonstream", split(&bytes, size)),
            &(),
        )
        .await
        .expect("extraction must succeed");

        let got: Vec<Item> = extracted
            .map(|r| r.expect("no item may fail"))
            .collect()
            .await;
        assert_eq!(got, items(), "failed at chunk size {size}");
    }
}

#[tokio::test]
async fn csv_round_trips_from_this_crates_encoder() {
    let bytes = encoded(StreamBodyAs::csv(futures::stream::iter(items()))).await;

    let extracted = <CsvStreamFrom<Item> as FromRequest<()>>::from_request(
        request("text/csv", vec![bytes]),
        &(),
    )
    .await
    .expect("extraction must succeed");

    let got: Vec<Item> = extracted
        .map(|r| r.expect("no item may fail"))
        .collect()
        .await;
    assert_eq!(got, items());
}

#[tokio::test]
async fn content_type_is_validated() {
    let table = [
        ("application/jsonstream", true),
        ("application/jsonstream; charset=utf-8", true),
        ("application/x-ndjson", true),
        ("APPLICATION/JSONSTREAM", true),
        ("application/json", false),
        ("text/csv", false),
    ];

    for (content_type, accepted) in table {
        let result = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(
            request(content_type, vec![b"{}\n".to_vec()]),
            &(),
        )
        .await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "unexpected verdict for {content_type}"
        );
    }
}

#[tokio::test]
async fn a_missing_content_type_is_rejected() {
    let req = Request::builder()
        .method("POST")
        .uri("/ingest")
        .body(Body::from("{}\n"))
        .unwrap();

    let result = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(req, &()).await;
    let rejection = result.expect_err("must be rejected");
    assert!(matches!(
        rejection,
        StreamBodyFromRejection::MissingContentType { .. }
    ));
    assert_eq!(rejection.status(), 415);
}

/// A rejection has to be decidable without reading a byte, or the handler never gets to stream.
#[tokio::test]
async fn a_too_large_declared_body_is_rejected_before_reading() {
    let req = Request::builder()
        .method("POST")
        .uri("/ingest")
        .header("content-type", "application/jsonstream")
        .header("content-length", "10000")
        .extension(StreamBodyFromOptions::new().max_body_len(16))
        .body(Body::from("{}\n"))
        .unwrap();

    let rejection = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(req, &())
        .await
        .expect_err("must be rejected");
    assert!(matches!(
        rejection,
        StreamBodyFromRejection::PayloadTooLarge { .. }
    ));
    assert_eq!(rejection.status(), 413);
}

/// JSON Lines records are independently framed, so one bad line is not terminal.
#[tokio::test]
async fn a_malformed_line_is_reported_and_the_stream_continues() {
    let body = b"{\"id\":1,\"name\":\"one\"}\nnot json\n{\"id\":2,\"name\":\"two\"}\n".to_vec();

    let extracted = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(
        request("application/jsonstream", vec![body]),
        &(),
    )
    .await
    .unwrap();

    let got: Vec<Result<Item, StreamBodyFromError>> = extracted.collect().await;
    assert_eq!(got.len(), 3);
    assert!(got[0].is_ok());
    assert!(got[1].is_err());
    assert_eq!(got[1].as_ref().err().unwrap().status(), 400);
    assert!(got[2].is_ok(), "a bad line must not end the stream");
}

/// A per-object cap must surface as 413 rather than as a generic decode failure.
#[tokio::test]
async fn an_oversized_object_reports_payload_too_large() {
    let body = format!("{{\"id\":1,\"name\":\"{}\"}}\n", "x".repeat(200)).into_bytes();

    let req = Request::builder()
        .method("POST")
        .uri("/ingest")
        .header("content-type", "application/jsonstream")
        .extension(StreamBodyFromOptions::new().max_obj_len(32))
        .body(Body::from(body))
        .unwrap();

    let extracted = <JsonNlStreamFrom<Item> as FromRequest<()>>::from_request(req, &())
        .await
        .unwrap();
    let got: Vec<Result<Item, StreamBodyFromError>> = extracted.collect().await;

    let err = got
        .iter()
        .find_map(|r| r.as_ref().err())
        .expect("the oversized object must be reported");
    assert_eq!(err.status(), 413);
}

/// The extractor must honour the app's body limit rather than quietly escaping it.
///
/// Goes through a real router and socket on purpose: `DefaultBodyLimit` is applied by a layer
/// that inserts an internal marker, so setting it as a request extension by hand does nothing
/// and would make this test pass without testing anything.
#[tokio::test]
async fn the_default_body_limit_is_honoured() {
    use axum::routing::post;
    use axum::Router;

    async fn ingest(mut items: JsonNlStreamFrom<Item>) -> String {
        let mut errors = 0;
        let mut statuses = Vec::new();
        while let Some(item) = items.next().await {
            if let Err(err) = item {
                errors += 1;
                statuses.push(err.status().as_u16());
            }
        }
        format!("{errors}:{statuses:?}")
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/ingest", post(ingest))
        .layer(DefaultBodyLimit::max(8));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let body = encoded(StreamBodyAs::json_nl(futures::stream::iter(items()))).await;
    assert!(body.len() > 8, "the body must actually exceed the limit");

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/ingest"))
        .header("content-type", "application/jsonstream")
        .body(body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let (errors, statuses) = response.split_once(':').unwrap();
    assert_ne!(
        errors, "0",
        "a tripped body limit must be reported: {response}"
    );
    assert!(
        statuses.contains("413"),
        "a tripped body limit must read as too-large, not as a malformed body: {response}"
    );
}

/// Protobuf and Arrow were the two formats with no extractor coverage, and they are exactly
/// the ones whose framing carries state across chunks.
mod binary {
    use super::*;

    #[derive(Clone, PartialEq, prost::Message)]
    struct Record {
        #[prost(uint32, tag = "1")]
        id: u32,
        #[prost(string, tag = "2")]
        name: String,
    }

    fn records() -> Vec<Record> {
        vec![
            Record {
                id: 1,
                name: "one".into(),
            },
            Record {
                id: 2,
                name: "two".into(),
            },
        ]
    }

    #[tokio::test]
    async fn protobuf_round_trips_at_every_chunk_boundary() {
        let bytes = encoded(StreamBodyAs::protobuf(futures::stream::iter(records()))).await;

        for size in 1..=bytes.len() {
            let extracted = <ProtobufStreamFrom<Record> as FromRequest<()>>::from_request(
                request("application/x-protobuf-stream", split(&bytes, size)),
                &(),
            )
            .await
            .expect("extraction must succeed");

            let got: Vec<Record> = extracted
                .map(|r| r.expect("no item may fail"))
                .collect()
                .await;
            assert_eq!(got, records(), "failed at chunk size {size}");
        }
    }

    /// A zero-length frame: every field at its default encodes to no bytes at all.
    #[tokio::test]
    async fn protobuf_round_trips_an_empty_message() {
        let items = vec![
            Record::default(),
            Record {
                id: 7,
                name: "after".into(),
            },
        ];
        let bytes = encoded(StreamBodyAs::protobuf(futures::stream::iter(items.clone()))).await;

        let extracted = <ProtobufStreamFrom<Record> as FromRequest<()>>::from_request(
            request("application/x-protobuf-stream", vec![bytes]),
            &(),
        )
        .await
        .unwrap();

        let got: Vec<Record> = extracted
            .map(|r| r.expect("no item may fail"))
            .collect()
            .await;
        assert_eq!(got, items);
    }

    #[tokio::test]
    async fn arrow_round_trips() {
        use arrow::array::{ArrayRef, Int32Array, RecordBatch};
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int32, false)]));
        let col: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
        let batch = RecordBatch::try_new(schema.clone(), vec![col]).unwrap();
        let batches = vec![batch];

        let bytes = encoded(StreamBodyAs::arrow_ipc(
            schema,
            futures::stream::iter(batches.clone()),
        ))
        .await;

        // Small chunk sizes only: an Arrow body is large and splitting at every offset would
        // dominate the suite without testing anything new.
        for size in [1, 3, 7, 64, 512] {
            if size > bytes.len() {
                continue;
            }
            let extracted = <ArrowIpcStreamFrom as FromRequest<()>>::from_request(
                request("application/vnd.apache.arrow.stream", split(&bytes, size)),
                &(),
            )
            .await
            .expect("extraction must succeed");

            let got: Vec<RecordBatch> = extracted
                .map(|r| r.expect("no batch may fail"))
                .collect()
                .await;
            assert_eq!(got, batches, "failed at chunk size {size}");
        }
    }
}
