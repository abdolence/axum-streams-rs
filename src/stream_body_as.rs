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
use std::sync::Arc;
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

    /// Reports every error passing through the body exactly once, before the buffering
    /// logic below can collapse or drop it.
    ///
    /// This is the single point every error reaches: each format forwards source errors
    /// verbatim and adds its own serialization errors to the same stream, so no format
    /// needs to know about reporting.
    fn report_errors(
        stream: BoxStream<'a, Result<axum::body::Bytes, axum::Error>>,
        options: &StreamBodyAsOptions,
    ) -> BoxStream<'a, Result<axum::body::Bytes, axum::Error>> {
        let handler = options.on_error.clone();

        if handler.is_none() && !cfg!(feature = "tracing") {
            return stream;
        }

        stream
            .inspect_err(move |err| {
                #[cfg(feature = "tracing")]
                tracing::error!(
                    target: "axum_streams",
                    error = %err,
                    "An error occurred while streaming an HTTP body; the response will be terminated abnormally"
                );

                if let Some(handler) = &handler {
                    handler(err);
                }
            })
            .boxed()
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
        let bytes_stream = Self::report_errors(
            stream_format.to_bytes_stream(boxed_stream, options),
            options,
        );

        match (options.buffering_ready_items, options.buffering_bytes) {
            (Some(buffering_ready_items), _) => bytes_stream
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
                let bytes_stream = bytes_stream.chain(futures::stream::once(
                    futures::future::ready(Ok(bytes::Bytes::new())),
                ));

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
            (None, None) => bytes_stream.map(|res| res.map(Frame::data)).boxed(),
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

/// A callback invoked for every error produced while streaming an HTTP body.
pub type StreamBodyAsErrorHandler = Arc<dyn Fn(&axum::Error) + Send + Sync + 'static>;

pub struct StreamBodyAsOptions {
    pub buffering_ready_items: Option<usize>,
    pub buffering_bytes: Option<usize>,
    pub content_type: Option<HttpHeaderValue>,
    pub on_error: Option<StreamBodyAsErrorHandler>,
}

impl StreamBodyAsOptions {
    pub fn new() -> Self {
        Self {
            buffering_ready_items: None,
            buffering_bytes: None,
            content_type: None,
            on_error: None,
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

    /// Registers a callback invoked for every error produced while streaming the body,
    /// covering both errors coming from your source stream and serialization errors
    /// produced by the format itself.
    ///
    /// The error still terminates the response; this is purely an observation hook.
    /// It does not replace the `tracing` feature: when that feature is enabled both the
    /// log event and this callback fire.
    pub fn on_error<F>(mut self, handler: F) -> Self
    where
        F: Fn(&axum::Error) + Send + Sync + 'static,
    {
        self.on_error = Some(Arc::new(handler));
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

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

    async fn drain(body: StreamBodyAs<'static>) {
        let _: Result<Vec<Bytes>, axum::Error> = body
            .into_response()
            .into_body()
            .into_data_stream()
            .try_collect()
            .await;
    }

    fn counting_options(counter: &Arc<AtomicUsize>) -> StreamBodyAsOptions {
        let counter = counter.clone();
        StreamBodyAsOptions::new().on_error(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[tokio::test]
    async fn test_on_error_called_once_unbuffered() {
        let counter = Arc::new(AtomicUsize::new(0));
        drain(failing_body(1, counting_options(&counter))).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_on_error_called_once_buffering_ready_items() {
        let counter = Arc::new(AtomicUsize::new(0));
        drain(failing_body(
            1,
            counting_options(&counter).buffering_ready_items(2),
        ))
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_on_error_called_once_buffering_bytes() {
        let counter = Arc::new(AtomicUsize::new(0));
        drain(failing_body(
            1,
            counting_options(&counter).buffering_bytes(3),
        ))
        .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_on_error_not_called_on_success() {
        let counter = Arc::new(AtomicUsize::new(0));
        // No item fails, so the handler must never run.
        drain(failing_body(usize::MAX, counting_options(&counter))).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_on_error_receives_error_message() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let options = StreamBodyAsOptions::new().on_error(move |err| {
            sink.lock().unwrap().push(err.to_string());
        });

        drain(failing_body(1, options)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].contains(FailingFormat::ERROR_MESSAGE));
    }

    #[tokio::test]
    async fn test_on_error_reports_errors_ready_chunks_collapses() {
        // Both errors land in the same `ready_chunks` batch, where only the first can reach
        // the client. The hook still observes both, which is the point of the hook.
        let counter = Arc::new(AtomicUsize::new(0));
        let counted = counter.clone();
        let stream = futures::stream::iter(vec![
            Ok("aaa".to_string()),
            Err(axum::Error::new("first")),
            Err(axum::Error::new("second")),
        ]);
        let body = StreamBodyAs::with_options(
            FailingFormat {
                fail_at_index: usize::MAX,
            },
            stream,
            StreamBodyAsOptions::new()
                .buffering_ready_items(4)
                .on_error(move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                }),
        );

        drain(body).await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
