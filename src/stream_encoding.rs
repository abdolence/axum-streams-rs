//! This crate's encoding pipeline: driving a format over a stream of items and producing body
//! bytes, in this crate's error type.
//!
//! The formats themselves live in [`http_streams_core`], so that the client-side crate encodes
//! byte-identical bodies from the same code. [`StreamingFormat`] stays here because it names
//! [`axum::Error`], which core has no way to name, and it is public and unsealed, so
//! third-party implementations of it must keep compiling. This module is where the two meet.
//!
//! [`StreamingFormat`]: crate::StreamingFormat

use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use http_streams_core::format::StreamFormatEncode;
use http_streams_core::{encode_stream, StreamError, StreamErrorKind};

/// Wraps an [`axum::Error`] so it can travel through the shared pipeline.
fn into_stream_error(err: axum::Error) -> StreamError {
    StreamError::new(StreamErrorKind::InputOutputError, Some(Box::new(err)), None)
}

/// Recovers the original [`axum::Error`] where there is one.
///
/// Errors coming from the caller's own source stream are wrapped by [`into_stream_error`] on
/// the way in and unwrapped again here, so that `on_error` and the HTTP layer see exactly the
/// error the caller produced rather than one nested two types deep. Errors raised by the
/// encoder itself have no such original and are wrapped once.
fn into_axum_error(err: StreamError) -> axum::Error {
    let (kind, source, message) = err.into_parts();

    match source {
        Some(boxed) => match boxed.downcast::<axum::Error>() {
            Ok(original) => *original,
            Err(other) => axum::Error::new(StreamError::new(kind, Some(other), message)),
        },
        None => axum::Error::new(StreamError::new(kind, None, message)),
    }
}

/// Encodes a stream of items into body bytes with `format`.
pub(crate) fn encode_items<'b, T, FMT>(
    format: &FMT,
    stream: BoxStream<'b, Result<T, axum::Error>>,
) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>>
where
    FMT: StreamFormatEncode<T>,
    FMT::Encoder: Send + 'b,
    T: Send + 'b,
{
    let source = Box::pin(stream.map_err(into_stream_error));
    encode_stream(source, format.encoder())
        .map_err(into_axum_error)
        .boxed()
}
