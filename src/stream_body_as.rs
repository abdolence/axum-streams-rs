use crate::stream_format::StreamingFormat;
use axum::body::{Body, HttpBody};
use axum::response::{IntoResponse, Response};
use bytes::BytesMut;
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::{HeaderMap, HeaderValue};
use http_body::Frame;
use std::fmt::Formatter;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct StreamBodyAs<'a> {
    stream: BoxStream<'a, Result<Frame<axum::body::Bytes>, axum::Error>>,
    headers: Option<HeaderMap>,
}

impl<'a> std::fmt::Debug for StreamBodyAs<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "StreamBodyWithFormat")
    }
}

impl<'a> StreamBodyAs<'a> {
    /// Create a new `StreamBodyWith` providing a stream of your objects in the specified format.
    pub fn new<S, T, FMT>(stream_format: FMT, stream: S) -> Self
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::with_options(stream_format, stream, StreamBodyAsOptions::new())
    }

    pub fn with_options<S, T, FMT>(
        stream_format: FMT,
        stream: S,
        options: StreamBodyAsOptions,
    ) -> Self
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = T> + 'a + Send,
    {
        Self {
            stream: Self::create_stream_frames(&stream_format, stream, options),
            headers: stream_format.http_response_trailers(),
        }
    }

    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: http::header::IntoHeaderName,
        V: Into<HeaderValue>,
    {
        let current_headers = self.headers.get_or_insert(HeaderMap::new());
        current_headers.append(key, value.into());
        self
    }

    fn create_stream_frames<S, T, FMT>(
        stream_format: &FMT,
        stream: S,
        options: StreamBodyAsOptions,
    ) -> BoxStream<'a, Result<Frame<axum::body::Bytes>, axum::Error>>
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = T> + 'a + Send,
    {
        match options.buffering_ready_items {
            Some(buffering_ready_items) => stream_format
                .to_bytes_stream(Box::pin(stream))
                .ready_chunks(buffering_ready_items)
                .map(|chunks| {
                    let mut buf = BytesMut::new();
                    for chunk in chunks {
                        buf.extend_from_slice(&chunk?);
                    }
                    Ok(Frame::data(buf.freeze()))
                })
                .boxed(),
            None => stream_format
                .to_bytes_stream(Box::pin(stream))
                .map(|res| res.map(Frame::data))
                .boxed(),
        }
    }
}

impl IntoResponse for StreamBodyAs<'static> {
    fn into_response(mut self) -> Response {
        let maybe_headers = self.headers.take();
        let mut response: Response<Body> = Response::new(Body::new(self));
        if let Some(headers) = maybe_headers {
            *response.headers_mut() = headers;
        }
        response
    }
}

impl<'a> HttpBody for StreamBodyAs<'a> {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

pub struct StreamBodyAsOptions {
    pub buffering_ready_items: Option<usize>,
}

impl StreamBodyAsOptions {
    pub fn new() -> Self {
        Self {
            buffering_ready_items: None,
        }
    }

    pub fn buffering_ready_items(mut self, ready_items: usize) -> Self {
        self.buffering_ready_items = Some(ready_items);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextStreamFormat;
    use bytes::Bytes;
    use futures::TryStreamExt;

    #[test]
    fn test_stream_body_as_options() {
        let options = StreamBodyAsOptions::new();
        assert_eq!(options.buffering_ready_items, None);

        let options = StreamBodyAsOptions::new().buffering_ready_items(10);
        assert_eq!(options.buffering_ready_items, Some(10));
    }

    #[tokio::test]
    async fn test_stream_body_as() {
        let stream = futures::stream::iter(vec!["First".to_string(), "Second".to_string()]).boxed();
        let stream_body_as = StreamBodyAs::new(TextStreamFormat::new(), stream);
        let response = stream_body_as.into_response();
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        let read = response.into_body().into_data_stream();
        let data: Vec<Bytes> = read.try_collect().await.unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], Bytes::from("First"));
        assert_eq!(data[1], Bytes::from("Second"));
    }

    #[tokio::test]
    async fn test_stream_body_as_buffering_item() {
        let stream = futures::stream::iter(vec![
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ])
        .boxed();
        let stream_body_as = StreamBodyAs::with_options(
            TextStreamFormat::new(),
            stream,
            StreamBodyAsOptions::new().buffering_ready_items(2),
        );
        let response = stream_body_as.into_response();
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        let read = response.into_body().into_data_stream();
        let data: Vec<Bytes> = read.try_collect().await.unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], Bytes::from("FirstSecond"));
        assert_eq!(data[1], Bytes::from("Third"));
    }
}
