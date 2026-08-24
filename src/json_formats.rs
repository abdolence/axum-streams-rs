//! JSON array and JSON Lines responses.
//!
//! The formats themselves live in [`http_streams_core`] and are re-exported here unchanged, so
//! that this crate and `reqwest-streams` produce byte-identical bodies from one implementation.
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

pub use http_streams_core::{JsonArrayStreamFormat, JsonNewLineStreamFormat};

impl<T, E> StreamingFormat<T> for JsonArrayStreamFormat<E>
where
    T: Serialize + Send + Sync + 'static,
    E: Serialize + Send + Sync + 'static,
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
                .unwrap_or_else(|| http::header::HeaderValue::from_static("application/json")),
        );
        Some(header_map)
    }

    fn format_name(&self) -> Option<&str> {
        Some("json_array")
    }
}

impl<T> StreamingFormat<T> for JsonNewLineStreamFormat
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

    fn http_response_headers(&self, _: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("application/jsonstream"),
        );
        Some(header_map)
    }

    fn format_name(&self) -> Option<&str> {
        Some("json_nl")
    }
}

impl<'a> crate::StreamBodyAs<'a> {
    pub fn json_array<S, T>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(
            JsonArrayStreamFormat::new(),
            stream.map(Ok::<T, axum::Error>),
        )
    }

    pub fn json_array_with_errors<S, T, E>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(JsonArrayStreamFormat::new(), stream)
    }

    pub fn json_array_with_envelope<S, T, EN>(stream: S, envelope: EN, array_field: &str) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
        EN: Serialize + Send + Sync + 'static,
    {
        Self::new(
            JsonArrayStreamFormat::with_envelope(envelope, array_field),
            stream.map(Ok::<T, axum::Error>),
        )
    }

    pub fn json_array_with_envelope_errors<S, T, E, EN>(
        stream: S,
        envelope: EN,
        array_field: &str,
    ) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
        EN: Serialize + Send + Sync + 'static,
    {
        Self::new(
            JsonArrayStreamFormat::with_envelope(envelope, array_field),
            stream,
        )
    }

    pub fn json_nl<S, T>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(
            JsonNewLineStreamFormat::new(),
            stream.map(Ok::<T, axum::Error>),
        )
    }

    pub fn json_nl_with_errors<S, T, E>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(JsonNewLineStreamFormat::new(), stream)
    }
}

impl StreamBodyAsOptions {
    pub fn json_array<'a, S, T>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        StreamBodyAs::with_options(
            JsonArrayStreamFormat::new(),
            stream.map(Ok::<T, axum::Error>),
            self,
        )
    }

    pub fn json_array_with_errors<'a, S, T, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(JsonArrayStreamFormat::new(), stream, self)
    }

    pub fn json_array_with_envelope<'a, S, T, EN>(
        self,
        stream: S,
        envelope: EN,
        array_field: &str,
    ) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
        EN: Serialize + Send + Sync + 'static,
    {
        StreamBodyAs::with_options(
            JsonArrayStreamFormat::with_envelope(envelope, array_field),
            stream.map(Ok::<T, axum::Error>),
            self,
        )
    }

    pub fn json_array_with_envelope_errors<'a, S, T, E, EN>(
        self,
        stream: S,
        envelope: EN,
        array_field: &str,
    ) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
        EN: Serialize + Send + Sync + 'static,
    {
        StreamBodyAs::with_options(
            JsonArrayStreamFormat::with_envelope(envelope, array_field),
            stream,
            self,
        )
    }

    pub fn json_nl<'a, S, T>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        StreamBodyAs::with_options(
            JsonNewLineStreamFormat::new(),
            stream.map(Ok::<T, axum::Error>),
            self,
        )
    }

    pub fn json_nl_with_errors<'a, S, T, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(JsonNewLineStreamFormat::new(), stream, self)
    }
}

/// A request body of JSON Lines, decoded into a stream of `T`.
///
/// ```rust,no_run
/// use axum::extract::DefaultBodyLimit;
/// use axum::{routing::post, Json, Router};
/// use axum_streams::{JsonNlStreamFrom, StreamBodyFromError};
/// use futures::StreamExt;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyItem {
///     field: String,
/// }
///
/// async fn ingest(mut items: JsonNlStreamFrom<MyItem>) -> Result<Json<u64>, StreamBodyFromError> {
///     let mut count = 0;
///     while let Some(item) = items.next().await {
///         // A malformed line is reported here, and the stream carries on to the next.
///         let _item = item?;
///         count += 1;
///     }
///     Ok(Json(count))
/// }
///
/// let app: Router = Router::new()
///     .route("/ingest", post(ingest))
///     .layer(DefaultBodyLimit::disable());
/// ```
pub type JsonNlStreamFrom<T> = crate::StreamBodyFrom<JsonNewLineStreamFormat, T>;

/// A request body of a JSON array, decoded into a stream of `T`.
///
/// Unlike JSON Lines, elements are not independently framed, so the first malformed element
/// ends the stream.
///
/// An envelope is not unwrapped on the way in: the decoder expects a bare array.
pub type JsonArrayStreamFrom<T> = crate::StreamBodyFrom<JsonArrayStreamFormat<()>, T>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures::stream;

    #[tokio::test]
    async fn serialize_json_array_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    JsonArrayStreamFormat::new(),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_json = serde_json::to_string(&test_stream_vec).unwrap();
        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/json")
        );

        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }

    #[tokio::test]
    async fn serialize_json_nl_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    JsonNewLineStreamFormat::new(),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_json = test_stream_vec
            .iter()
            .map(|item| serde_json::to_string(item).unwrap())
            .collect::<Vec<String>>()
            .join("\n")
            + "\n";

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/jsonstream")
        );

        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }

    #[tokio::test]
    async fn serialize_json_array_stream_with_envelope_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestItemStructure {
            foo: String,
        }

        #[derive(Debug, Clone, Serialize)]
        struct TestEnvelopeStructure {
            envelope_field: String,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            my_array: Vec<TestItemStructure>,
        }

        let test_stream_vec = vec![
            TestItemStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let test_envelope = TestEnvelopeStructure {
            envelope_field: "test_envelope".to_string(),
            my_array: Vec::new(),
        };

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    JsonArrayStreamFormat::with_envelope(test_envelope, "my_array"),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_envelope = TestEnvelopeStructure {
            envelope_field: "test_envelope".to_string(),
            my_array: test_stream_vec.clone(),
        };

        let expected_json = serde_json::to_string(&expected_envelope).unwrap();
        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/json")
        );

        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }

    #[tokio::test]
    async fn serialize_json_array_stream_with_empty_envelope_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestItemStructure {
            foo: String,
        }

        #[derive(Debug, Clone, Serialize)]
        struct TestEnvelopeStructure {
            #[serde(skip_serializing_if = "Vec::is_empty")]
            my_array: Vec<TestItemStructure>,
        }

        let test_stream_vec = vec![
            TestItemStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let test_envelope = TestEnvelopeStructure {
            my_array: Vec::new(),
        };

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    JsonArrayStreamFormat::with_envelope(test_envelope, "my_array"),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_envelope = TestEnvelopeStructure {
            my_array: test_stream_vec.clone(),
        };

        let expected_json = serde_json::to_string(&expected_envelope).unwrap();
        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/json")
        );

        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }
}
