//! Raw UTF-8 text responses.
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

pub use http_streams_core::TextStreamFormat;

impl StreamingFormat<String> for TextStreamFormat {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<String, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        encode_items(self, stream)
    }

    fn http_response_headers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            options.content_type.clone().unwrap_or_else(|| {
                http::header::HeaderValue::from_static("text/plain; charset=utf-8")
            }),
        );
        Some(header_map)
    }

    fn format_name(&self) -> Option<&str> {
        Some("text")
    }
}
impl<'a> StreamBodyAs<'a> {
    pub fn text<S>(stream: S) -> Self
    where
        S: Stream<Item = String> + 'a + Send,
    {
        Self::new(
            TextStreamFormat::new(),
            stream.map(Ok::<String, axum::Error>),
        )
    }

    pub fn text_with_errors<S, E>(stream: S) -> Self
    where
        S: Stream<Item = Result<String, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(TextStreamFormat::new(), stream)
    }
}

impl StreamBodyAsOptions {
    pub fn text<'a, S>(self, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = String> + 'a + Send,
    {
        StreamBodyAs::with_options(
            TextStreamFormat::new(),
            stream.map(Ok::<String, axum::Error>),
            self,
        )
    }

    pub fn text_with_errors<'a, S, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = Result<String, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(TextStreamFormat::new(), stream, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures::stream;

    #[tokio::test]
    async fn serialize_text_stream_format() {
        let test_stream_vec = vec![
            String::from("bar1"),
            String::from("bar2"),
            String::from("bar3"),
            String::from("bar4"),
            String::from("bar5"),
            String::from("bar6"),
            String::from("bar7"),
            String::from("bar8"),
            String::from("bar9"),
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    TextStreamFormat::new(),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_text_buf: Vec<u8> = test_stream_vec
            .iter()
            .flat_map(|obj| {
                let obj_vec = obj.as_bytes().to_vec();
                obj_vec
            })
            .collect();

        let res = client.get("/").send().await.unwrap();
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body, expected_text_buf);
    }
}
