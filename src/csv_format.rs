//! CSV responses.
//!
//! The format itself lives in [`http_streams_core`] and is re-exported here unchanged, so that
//! this crate and `reqwest-streams` produce byte-identical bodies from one implementation.
//! What remains here is the [`StreamingFormat`] shim — that trait names [`axum::Error`], which
//! core cannot — and the response headers, which are an HTTP concern rather than a framing one.

use crate::stream_body_as::StreamBodyAsOptions;
use crate::stream_encoding::encode_items;
use crate::stream_format::StreamingFormat;
use crate::StreamBodyAs;
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;
use serde::Serialize;

pub use http_streams_core::CsvStreamFormat;

impl<T> StreamingFormat<T> for CsvStreamFormat
where
    T: Serialize + Send + Sync + 'static,
{
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<T, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        encode_items(self, stream)
    }

    fn http_response_headers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            options
                .content_type
                .clone()
                .unwrap_or_else(|| http::header::HeaderValue::from_static("text/csv")),
        );
        Some(header_map)
    }

    fn format_name(&self) -> Option<&str> {
        Some("csv")
    }
}
impl<'a> StreamBodyAs<'a> {
    pub fn csv<S, T>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(CsvStreamFormat::default(), stream.map(Ok::<T, axum::Error>))
    }

    pub fn csv_with_errors<S, T, E>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error> + 'static,
    {
        Self::new(CsvStreamFormat::new(false, b','), stream)
    }
}

impl StreamBodyAsOptions {
    pub fn csv<'a, S, T>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        StreamBodyAs::with_options(
            CsvStreamFormat::new(false, b','),
            stream.map(Ok::<T, axum::Error>),
            self,
        )
    }

    pub fn csv_with_errors<'a, S, T, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error> + 'static,
    {
        StreamBodyAs::with_options(CsvStreamFormat::new(false, b','), stream, self)
    }
}

/// A CSV request body, decoded into a stream of `T`.
///
/// Records are deserialised positionally; a header row is consumed and discarded. A malformed
/// row is reported and the stream carries on, because rows are independently framed.
///
/// The default configuration is a comma delimiter with a header row. For anything else, attach
/// the format to the route:
///
/// ```rust,no_run
/// use axum::extract::DefaultBodyLimit;
/// use axum::{routing::post, Extension, Router};
/// use axum_streams::{CsvStreamFormat, CsvStreamFrom, StreamBodyFromConfig};
/// use futures::StreamExt;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Row {
///     id: u32,
/// }
///
/// async fn ingest(mut rows: CsvStreamFrom<Row>) -> String {
///     let mut count = 0;
///     while let Some(row) = rows.next().await {
///         if row.is_ok() {
///             count += 1;
///         }
///     }
///     format!("{count}")
/// }
///
/// let app: Router = Router::new()
///     .route("/ingest", post(ingest))
///     .layer(Extension(StreamBodyFromConfig::new(
///         CsvStreamFormat::new(true, b';'),
///     )))
///     .layer(DefaultBodyLimit::disable());
/// ```
pub type CsvStreamFrom<T> = crate::StreamBodyFrom<CsvStreamFormat, T>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures::stream;
    use std::ops::Add;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn serialize_csv_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo1: String,
            foo2: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo1: "bar1".to_string(),
                foo2: "bar2".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    CsvStreamFormat::new(false, b'.').with_delimiter(b','),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_csv = test_stream_vec
            .iter()
            .map(|item| format!("{},{}", item.foo1, item.foo2))
            .collect::<Vec<String>>()
            .join("\n")
            .add("\n");

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("text/csv")
        );
        let body = res.text().await.unwrap();

        assert_eq!(body, expected_csv);
    }

    #[tokio::test]
    async fn serialize_csv_stream_format_error_is_reported() {
        // The scenario from https://github.com/abdolence/axum-streams-rs/issues/63: `csv`
        // refuses to write headers for a struct containing a sequence, and the resulting
        // error used to vanish without a trace.
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo1: String,
            nested: Vec<String>,
        }

        let test_stream = Box::pin(stream::iter(vec![TestOutputStructure {
            foo1: "bar1".to_string(),
            nested: vec!["a".to_string(), "b".to_string()],
        }]));

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();

        let app = Router::new().route(
            "/",
            get(|| async move {
                StreamBodyAs::with_options(
                    // `has_headers` is what triggers the failure in the issue.
                    CsvStreamFormat::default(),
                    test_stream.map(Ok::<_, axum::Error>),
                    StreamBodyAsOptions::new().on_error(move |err| {
                        sink.lock().unwrap().push(err.to_string());
                    }),
                )
            }),
        );

        let client = TestClient::new(app).await;
        // The response is aborted, so the request itself fails: this is exactly what the
        // issue reporter saw, with no indication anywhere of the underlying cause.
        match client.get("/").send().await {
            Ok(res) => {
                let _ = res.text().await;
            }
            Err(err) => assert!(err.is_request() || err.is_body()),
        }

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(
            captured[0].contains("cannot serialize sequence container inside struct"),
            "unexpected error reported: {}",
            captured[0]
        );
    }
}
