//! Receiving a streamed request body.
//!
//! The mirror of [`StreamBodyAs`]: instead of encoding a stream of items into a response body,
//! this decodes a request body into a stream of items, so a handler can process an upload of
//! any size without holding it in memory.

use crate::StreamBodyAsProgressHandler;
use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use axum::RequestExt;
use futures::stream::BoxStream;
use futures::{Stream, TryStreamExt};
use http::StatusCode;
use http_streams_core::format::{DecodeOptions, DefaultFormat, StreamFormatDecode};
use http_streams_core::{
    count_bytes, decode_stream, instrument, ContentType, Counting, Direction, Progress,
    ProgressOptions, Side, StreamContext, StreamError, StreamErrorKind,
};
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

/// A server reads a body it did not produce, so an unbounded object limit would be an invitation.
/// The client side defaults to no limit, which is defensible there because the peer was chosen.
pub const DEFAULT_MAX_OBJ_LEN: usize = 1024 * 1024;

/// Options for receiving a streamed request body.
///
/// Supply them to a route with `.layer(Extension(StreamBodyFromOptions::new()…))`, which is how
/// axum configures its own extractors.
#[derive(Clone)]
#[non_exhaustive]
pub struct StreamBodyFromOptions {
    /// Maximum length of a single decoded object. One MiB by default.
    pub max_obj_len: usize,
    /// Initial capacity of the read buffer.
    pub buf_capacity: usize,
    /// Reject a request whose declared `Content-Length` exceeds this, before reading anything.
    pub max_body_len: Option<u64>,
    /// Whether to require a `Content-Type` the format recognises. On by default.
    pub validate_content_type: bool,
    /// Invoked for every progress report as the body is read.
    pub on_progress: Option<StreamBodyAsProgressHandler>,
    /// How often to report interim progress. One second by default.
    pub progress_interval: Option<Duration>,
    /// Additionally report progress every N items.
    pub progress_items: Option<u64>,
}

impl std::fmt::Debug for StreamBodyFromOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamBodyFromOptions")
            .field("max_obj_len", &self.max_obj_len)
            .field("buf_capacity", &self.buf_capacity)
            .field("max_body_len", &self.max_body_len)
            .field("validate_content_type", &self.validate_content_type)
            .field("progress_interval", &self.progress_interval)
            .field("progress_items", &self.progress_items)
            .finish_non_exhaustive()
    }
}

impl Default for StreamBodyFromOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamBodyFromOptions {
    /// Default options.
    pub fn new() -> Self {
        Self {
            max_obj_len: DEFAULT_MAX_OBJ_LEN,
            buf_capacity: http_streams_core::DEFAULT_BUF_CAPACITY,
            max_body_len: None,
            validate_content_type: true,
            on_progress: None,
            progress_interval: Some(http_streams_core::DEFAULT_PROGRESS_INTERVAL),
            progress_items: None,
        }
    }

    /// Sets the maximum length of a single decoded object.
    pub fn max_obj_len(mut self, len: usize) -> Self {
        self.max_obj_len = len;
        self
    }

    /// Sets the initial capacity of the read buffer.
    pub fn buf_capacity(mut self, capacity: usize) -> Self {
        self.buf_capacity = capacity;
        self
    }

    /// Rejects a request whose declared `Content-Length` exceeds `len`.
    ///
    /// Only a pre-check on the header, so it does nothing for a chunked request, which sends
    /// no `Content-Length` at all. The real backstop is axum's
    /// [`DefaultBodyLimit`](axum::extract::DefaultBodyLimit), which this extractor honours and
    /// which applies at 2 MiB even when no limit was configured explicitly.
    pub fn max_body_len(mut self, len: u64) -> Self {
        self.max_body_len = Some(len);
        self
    }

    /// Whether to require a `Content-Type` the format recognises.
    ///
    /// Turn it off for producers that send none, such as `curl --data-binary @-`.
    pub fn validate_content_type(mut self, validate: bool) -> Self {
        self.validate_content_type = validate;
        self
    }

    /// Registers a callback receiving progress snapshots as the body is read.
    pub fn on_progress<F>(mut self, handler: F) -> Self
    where
        F: Fn(&crate::StreamProgress) + Send + Sync + 'static,
    {
        self.on_progress = Some(std::sync::Arc::new(handler));
        self
    }

    /// Reports progress at most once per `interval`.
    pub fn progress_interval(mut self, interval: Duration) -> Self {
        self.progress_interval = Some(interval);
        self
    }

    /// Additionally reports progress every `items` items.
    pub fn progress_items(mut self, items: u64) -> Self {
        self.progress_items = Some(items);
        self
    }

    fn progress_options(&self) -> ProgressOptions {
        let mut opts = ProgressOptions::new();
        opts.on_progress = self.on_progress.clone();
        opts.progress_interval = self.progress_interval;
        opts.progress_items = self.progress_items;
        opts
    }

    fn decode_options(&self) -> DecodeOptions {
        DecodeOptions::new()
            .max_obj_len(self.max_obj_len)
            .buf_capacity(self.buf_capacity)
    }
}

/// Carries a configured format to a [`StreamBodyFrom`] extractor.
///
/// The extractor is built by axum rather than by you, so there is nowhere to pass constructor
/// arguments. Attach the format you already know how to build to the route instead, with
/// `.layer(Extension(StreamBodyFromConfig::new(my_format)))`. This is the same idiom axum uses
/// for its own extractors: [`DefaultBodyLimit`](axum::extract::DefaultBodyLimit) is an
/// extension read back the same way.
///
/// Without one, the format is built by [`DefaultFormat::default_format`]. See
/// [`CsvStreamFrom`](crate::CsvStreamFrom) for a worked example.
#[derive(Debug, Clone)]
pub struct StreamBodyFromConfig<FMT>(pub FMT);

impl<FMT> StreamBodyFromConfig<FMT> {
    /// Wraps a configured format.
    pub fn new(format: FMT) -> Self {
        Self(format)
    }
}

/// An error decoding one record of a streamed request body.
///
/// Reaches the handler as a stream item rather than as a rejection, because by the time it
/// happens the handler already owns the stream. Returning it with `?` produces the right
/// status, provided the handler has not begun writing its response.
#[derive(Debug)]
pub struct StreamBodyFromError(StreamError);

impl StreamBodyFromError {
    /// The underlying error.
    pub fn inner(&self) -> &StreamError {
        &self.0
    }

    /// The status this error should produce.
    pub fn status(&self) -> StatusCode {
        match self.0.kind() {
            StreamErrorKind::MaxLenReachedError | StreamErrorKind::MaxBodyLenReachedError => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl std::fmt::Display for StreamBodyFromError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for StreamBodyFromError {}

impl IntoResponse for StreamBodyFromError {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

/// Why a streamed request body was refused before any of it was read.
///
/// Only failures knowable without reading a byte can be rejections: the extractor has to return
/// before the body arrives, or the handler never gets to stream it. Everything else, malformed
/// records included, reaches the handler as a [`StreamBodyFromError`] item.
#[derive(Debug)]
#[non_exhaustive]
pub enum StreamBodyFromRejection {
    /// The `Content-Type` is not one this format reads.
    UnsupportedMediaType {
        /// What the format emits, as a hint.
        expected: &'static str,
        /// What arrived.
        found: String,
    },
    /// No `Content-Type` at all.
    MissingContentType {
        /// What the format emits, as a hint.
        expected: &'static str,
    },
    /// The declared `Content-Length` exceeds the configured maximum.
    PayloadTooLarge {
        /// What the request declared.
        content_length: u64,
        /// What was allowed.
        max_body_len: u64,
    },
}

impl StreamBodyFromRejection {
    /// The status this rejection produces.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::UnsupportedMediaType { .. } | Self::MissingContentType { .. } => {
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            }
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        }
    }
}

impl std::fmt::Display for StreamBodyFromRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMediaType { expected, found } => write!(
                f,
                "Unsupported content type `{found}`, expected something like `{expected}`"
            ),
            Self::MissingContentType { expected } => write!(
                f,
                "Missing content type, expected something like `{expected}`"
            ),
            Self::PayloadTooLarge {
                content_length,
                max_body_len,
            } => write!(
                f,
                "Body of {content_length} bytes exceeds the limit of {max_body_len}"
            ),
        }
    }
}

impl std::error::Error for StreamBodyFromRejection {}

impl IntoResponse for StreamBodyFromRejection {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

/// A request body decoded into a stream of items.
///
/// Use one of the format aliases rather than naming this directly: `JsonNlStreamFrom`,
/// `JsonArrayStreamFrom`, `CsvStreamFrom`, `ProtobufStreamFrom`, `ArrowIpcStreamFrom`. Each
/// carries a worked example.
///
/// Note that a route taking one needs `.layer(DefaultBodyLimit::disable())`: a streaming upload
/// is exactly what axum's default body limit exists to stop, so a route that wants one has to
/// say so. This extractor honours that limit rather than quietly escaping it.
pub struct StreamBodyFrom<FMT, T> {
    stream: BoxStream<'static, Result<T, StreamBodyFromError>>,
    // `fn() -> FMT` so the format's own auto traits are not dragged into this type.
    _fmt: PhantomData<fn() -> FMT>,
}

impl<FMT, T> std::fmt::Debug for StreamBodyFrom<FMT, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamBodyFrom").finish_non_exhaustive()
    }
}

impl<FMT, T> StreamBodyFrom<FMT, T> {
    /// The underlying stream.
    pub fn into_inner(self) -> BoxStream<'static, Result<T, StreamBodyFromError>> {
        self.stream
    }
}

impl<FMT, T> Stream for StreamBodyFrom<FMT, T> {
    type Item = Result<T, StreamBodyFromError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
    }
}

/// Turns a body-stream error into a core error, keeping a tripped body limit distinguishable
/// from an ordinary transport failure so the handler can answer 413 rather than 400.
fn body_error(err: axum::Error) -> std::io::Error {
    let boxed = err.into_inner();

    // Walked rather than checked one level down: how deeply hyper and axum nest the cause is
    // not part of anything's contract, and a limit reported as a plain I/O error would answer
    // 400 for a request that is really 413.
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(boxed.as_ref());
    while let Some(err) = cause {
        if err.is::<http_body_util::LengthLimitError>() {
            return std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                LimitTripped(boxed.to_string()),
            );
        }
        cause = err.source();
    }

    std::io::Error::other(boxed)
}

/// Marker for a body that hit its limit, recognised again when classifying stream errors.
#[derive(Debug)]
struct LimitTripped(String);

impl std::fmt::Display for LimitTripped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LimitTripped {}

/// Whether this error is the app's body limit being hit, rather than an ordinary transport
/// failure.
///
/// Follows the marker planted by [`body_error`] down the chain the error travelled: it was
/// wrapped in an [`std::io::Error`] to cross [`StreamReader`](tokio_util::io::StreamReader),
/// and `StreamError`'s `From<io::Error>` then boxed that as its source. Matching the type
/// rather than the message keeps this from silently breaking when a dependency rewords itself.
fn is_limit_tripped(err: &StreamError) -> bool {
    if !matches!(err.kind(), StreamErrorKind::InputOutputError) {
        return false;
    }

    err.source()
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .and_then(|io_err| io_err.get_ref())
        .is_some_and(|inner| inner.is::<LimitTripped>())
}

fn to_from_error(err: StreamError) -> StreamBodyFromError {
    // A tripped body limit is a 413, not the 400 an I/O error would otherwise produce.
    if is_limit_tripped(&err) {
        return StreamBodyFromError(StreamError::new(
            StreamErrorKind::MaxBodyLenReachedError,
            None,
            Some("Body length limit exceeded".into()),
        ));
    }
    StreamBodyFromError(err)
}

fn declared_content_length(headers: &http::HeaderMap) -> Option<u64> {
    headers
        .get(http::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

impl<S, FMT, T> FromRequest<S> for StreamBodyFrom<FMT, T>
where
    S: Send + Sync,
    FMT: StreamFormatDecode<T> + DefaultFormat + Clone + Send + Sync + 'static,
    FMT::Framer: 'static,
    FMT::Parser: 'static,
    FMT::Frame: 'static,
    T: Send + 'static,
{
    type Rejection = StreamBodyFromRejection;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let options = req
            .extensions()
            .get::<StreamBodyFromOptions>()
            .cloned()
            .unwrap_or_default();

        let format = req
            .extensions()
            .get::<StreamBodyFromConfig<FMT>>()
            .map(|config| config.0.clone())
            .unwrap_or_else(FMT::default_format);

        if options.validate_content_type {
            let raw = req
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());

            match raw {
                None => {
                    return Err(StreamBodyFromRejection::MissingContentType {
                        expected: format.default_content_type(),
                    })
                }
                Some(raw) if !format.accepts_content_type(&ContentType::parse(raw)) => {
                    return Err(StreamBodyFromRejection::UnsupportedMediaType {
                        expected: format.default_content_type(),
                        found: raw.to_string(),
                    })
                }
                Some(_) => {}
            }
        }

        if let Some(max) = options.max_body_len {
            if let Some(declared) = declared_content_length(req.headers()) {
                if declared > max {
                    return Err(StreamBodyFromRejection::PayloadTooLarge {
                        content_length: declared,
                        max_body_len: max,
                    });
                }
            }
        }

        let context = StreamContext::new(
            format.format_name().to_string(),
            Direction::Request,
            Side::Server,
        )
        .content_length(declared_content_length(req.headers()))
        .max_obj_len(options.max_obj_len)
        .buf_capacity(options.buf_capacity);

        let progress = Progress::new(&context, &options.progress_options());

        // Opting in to the app's body limit rather than quietly escaping it. An extractor that
        // reads a body without honouring `DefaultBodyLimit` is how advisories start.
        let body = req.into_limited_body();
        let bytes = count_bytes(body.into_data_stream().map_err(body_error), &progress);

        let decode_options = options.decode_options();
        let items = decode_stream(
            bytes,
            format.framer(&decode_options),
            format.parser(),
            &decode_options,
        )
        .map_err(to_from_error);

        let stream = Box::pin(instrument(Box::pin(items), progress, Counting::Items));

        Ok(Self {
            stream,
            _fmt: PhantomData,
        })
    }
}
