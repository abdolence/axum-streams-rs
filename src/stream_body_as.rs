use crate::stream_format::StreamingFormat;
use axum::body::{Body, HttpBody};
use axum::response::{IntoResponse, Response};
use bytes::BytesMut;
use futures::stream::BoxStream;
use futures::StreamExt;
use futures::{Stream, TryStreamExt};
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
    pub fn new<S, T, FMT, E>(stream_format: FMT, stream: S) -> Self
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::with_options(stream_format, stream, StreamBodyAsOptions::new())
    }

    pub fn with_options<S, T, FMT, E>(
        stream_format: FMT,
        stream: S,
        options: StreamBodyAsOptions,
    ) -> Self
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self {
            stream: Self::create_stream_frames(&stream_format, stream, &options),
            headers: stream_format.http_response_headers(&options),
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

    fn create_stream_frames<S, T, FMT, E>(
        stream_format: &FMT,
        stream: S,
        options: &StreamBodyAsOptions,
    ) -> BoxStream<'a, Result<Frame<axum::body::Bytes>, axum::Error>>
    where
        FMT: StreamingFormat<T>,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        let boxed_stream = Box::pin(stream.map_err(|e| e.into()));
        match (options.buffering_ready_items, options.buffering_bytes) {
            (Some(buffering_ready_items), _) => stream_format
                .to_bytes_stream(boxed_stream, options)
                .ready_chunks(buffering_ready_items)
                .map(|chunks| {
                    let mut buf = BytesMut::new();
                    for chunk in chunks {
                        buf.extend_from_slice(&chunk?);
                    }
                    Ok(Frame::data(buf.freeze()))
                })
                .boxed(),
            (_, Some(buffering_bytes)) => {
                let bytes_stream = stream_format.to_bytes_stream(boxed_stream, options).chain(
                    futures::stream::once(futures::future::ready(Ok(bytes::Bytes::new()))),
                );

                bytes_stream
                    .scan(
                        (BytesMut::with_capacity(buffering_bytes), false),
                        move |(current_buffer, errored), maybe_bytes| {
                            futures::future::ready(if *errored {
                                None
                            } else {
                                match maybe_bytes {
                                    Ok(bytes) if bytes.is_empty() => {
                                        Some(vec![Ok(Frame::data(current_buffer.split().freeze()))])
                                    }
                                    Ok(bytes) => {
                                        let mut frames = Vec::new();
                                        current_buffer.extend_from_slice(&bytes);
                                        while current_buffer.len() >= buffering_bytes {
                                            let buffer =
                                                current_buffer.split_to(buffering_bytes).freeze();
                                            frames.push(Ok(Frame::data(buffer)));
                                        }
                                        Some(frames)
                                    }
                                    // Propagate the error instead of ending the stream: returning
                                    // `None` here made a failure indistinguishable from a clean EOF,
                                    // so clients silently accepted a truncated body. Buffered bytes
                                    // are dropped and the stream stops, so no data frame can follow
                                    // the error via the trailing flush marker below.
                                    Err(e) => {
                                        *errored = true;
                                        current_buffer.clear();
                                        Some(vec![Err(e)])
                                    }
                                }
                            })
                        },
                    )
                    .flat_map(|res| futures::stream::iter(res).boxed())
                    .boxed()
            }
            (None, None) => stream_format
                .to_bytes_stream(boxed_stream, options)
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

pub type HttpHeaderValue = http::header::HeaderValue;

pub struct StreamBodyAsOptions {
    pub buffering_ready_items: Option<usize>,
    pub buffering_bytes: Option<usize>,
    pub content_type: Option<HttpHeaderValue>,
}

impl StreamBodyAsOptions {
    pub fn new() -> Self {
        Self {
            buffering_ready_items: None,
            buffering_bytes: None,
            content_type: None,
        }
    }

    pub fn buffering_ready_items(mut self, ready_items: usize) -> Self {
        self.buffering_ready_items = Some(ready_items);
        self
    }

    pub fn buffering_bytes(mut self, ready_bytes: usize) -> Self {
        self.buffering_bytes = Some(ready_bytes);
        self
    }

    pub fn content_type(mut self, content_type: HttpHeaderValue) -> Self {
        self.content_type = Some(content_type);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "text")]
    use crate::TextStreamFormat;
    use bytes::Bytes;
    use futures::TryStreamExt;

    /// A format that fails on a chosen item, so the error paths can be exercised without
    /// depending on any of the optional format features.
    struct FailingFormat {
        fail_at_index: usize,
    }

    impl FailingFormat {
        const ERROR_MESSAGE: &'static str = "simulated serialize failure";
    }

    impl StreamingFormat<String> for FailingFormat {
        fn to_bytes_stream<'a, 'b>(
            &'a self,
            stream: BoxStream<'b, Result<String, axum::Error>>,
            _: &'a StreamBodyAsOptions,
        ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
            let fail_at_index = self.fail_at_index;
            Box::pin(
                stream
                    .enumerate()
                    .map(move |(index, obj_res)| match obj_res {
                        Err(e) => Err(e),
                        Ok(_) if index == fail_at_index => {
                            Err(axum::Error::new(Self::ERROR_MESSAGE))
                        }
                        Ok(obj) => Ok(axum::body::Bytes::from(obj)),
                    }),
            )
        }

        fn http_response_headers(&self, _: &StreamBodyAsOptions) -> Option<HeaderMap> {
            None
        }
    }

    fn failing_body(fail_at_index: usize, options: StreamBodyAsOptions) -> StreamBodyAs<'static> {
        let stream = futures::stream::iter(vec![
            "aaa".to_string(),
            "bbb".to_string(),
            "ccc".to_string(),
        ]);
        StreamBodyAs::with_options(
            FailingFormat { fail_at_index },
            stream.map(Ok::<_, axum::Error>),
            options,
        )
    }

    #[test]
    fn test_stream_body_as_options() {
        let options = StreamBodyAsOptions::new();
        assert_eq!(options.buffering_ready_items, None);

        let options = StreamBodyAsOptions::new().buffering_ready_items(10);
        assert_eq!(options.buffering_ready_items, Some(10));
    }

    #[cfg(feature = "text")]
    #[tokio::test]
    async fn test_stream_body_as() {
        let stream = futures::stream::iter(vec!["First".to_string(), "Second".to_string()]).boxed();
        let stream_body_as =
            StreamBodyAs::new(TextStreamFormat::new(), stream.map(Ok::<_, axum::Error>));
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

    #[cfg(feature = "text")]
    #[tokio::test]
    async fn test_stream_body_as_buffering_items() {
        let stream = futures::stream::iter(vec![
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ])
        .boxed();
        let stream_body_as = StreamBodyAs::with_options(
            TextStreamFormat::new(),
            stream.map(Ok::<_, axum::Error>),
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

    #[cfg(feature = "text")]
    #[tokio::test]
    async fn test_stream_body_as_buffering_bytes() {
        let stream = futures::stream::iter(vec![
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ])
        .boxed();
        let stream_body_as = StreamBodyAs::with_options(
            TextStreamFormat::new(),
            stream.map(Ok::<_, axum::Error>),
            StreamBodyAsOptions::new().buffering_bytes(3),
        );
        let response = stream_body_as.into_response();
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        let read = response.into_body().into_data_stream();
        let data: Vec<Bytes> = read.try_collect().await.unwrap();
        assert_eq!(data.len(), 6);
        assert_eq!(data[0], Bytes::from("Fir"));
        assert_eq!(data[1], Bytes::from("stS"));
        assert_eq!(data[2], Bytes::from("eco"));
        assert_eq!(data[3], Bytes::from("ndT"));
        assert_eq!(data[4], Bytes::from("hir"));
        assert_eq!(data[5], Bytes::from("d"));
    }

    #[tokio::test]
    async fn test_buffering_bytes_error_is_not_swallowed() {
        let body = failing_body(1, StreamBodyAsOptions::new().buffering_bytes(3));
        let collected: Result<Vec<Bytes>, axum::Error> = body
            .into_response()
            .into_body()
            .into_data_stream()
            .try_collect()
            .await;

        let err = collected.expect_err("the error must reach the client, not truncate the body");
        assert!(err.to_string().contains(FailingFormat::ERROR_MESSAGE));
    }

    #[tokio::test]
    async fn test_buffering_bytes_stops_after_error() {
        let body = failing_body(1, StreamBodyAsOptions::new().buffering_bytes(3));
        let mut stream = body.into_response().into_body().into_data_stream();

        // "aaa" fills the buffer exactly and is flushed as one frame.
        assert_eq!(stream.next().await.unwrap().unwrap(), Bytes::from("aaa"));
        assert!(stream.next().await.unwrap().is_err());
        // No trailing flush frame may follow the error.
        assert!(stream.next().await.is_none());
    }
}
