#![allow(unused_parens, clippy::new_without_default)]
#![forbid(unsafe_code)]

/// Axum streaming body support that [`HttpBody`] created from an async ['Stream'] for different formats:
/// - JSON array stream format
/// - JSON Lines (NL/NewLines) format
/// - CSV stream format
///
/// [JSON Streaming](https://en.wikipedia.org/wiki/JSON_streaming) is a term referring to streaming a
/// stream of element as independent JSON objects as a continuous HTTP request or response.
///
/// This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
/// and want to avoid huge memory allocations to store on the server side.
///
/// # Example
///
/// ```rust
/// use futures_util::stream::BoxStream;
/// use axum::{
///     Router,
///     routing::get,
///     http::{StatusCode, header::CONTENT_TYPE},
///     response::{Response, IntoResponse},
/// };
/// use axum_streams::*;
/// use serde::Serialize;
///
/// #[derive(Debug, Clone, Serialize)]
/// struct MyTestStructure {
///     some_test_field: String
/// }
///
/// // Your possibly stream of objects
/// fn my_source_stream() -> BoxStream<'static, MyTestStructure> {
///     // Simulating a stream with a plain vector and throttling to show how it works
///     use tokio_stream::StreamExt;
///     Box::pin(futures::stream::iter(vec![
///         MyTestStructure {
///             some_test_field: "test1".to_string()
///         }; 1000
///     ]).throttle(std::time::Duration::from_millis(50)))
/// }
///
/// // Route implementation:
/// async fn test_json_array_stream() -> impl IntoResponse {
///     StreamBodyWithFormat ::new(JsonArrayStreamFormat::new(), my_source_stream())
/// }
/// async fn test_json_nl_stream() -> impl IntoResponse {
///     StreamBodyWithFormat ::new(JsonNewLineStreamFormat::new(), my_source_stream())
/// }
/// async fn test_csv_stream() -> impl IntoResponse {
///     StreamBodyWithFormat ::new(CsvStreamFormat::new(
///         true, // with_header
///         b',' // CSV delimiter
///     ), my_source_stream())
/// }
///
/// ```
mod stream_format;

mod stream_body_with;
pub use self::stream_body_with::StreamBodyWithFormat;

#[cfg(feature = "json")]
mod json_formats;
#[cfg(feature = "json")]
pub use json_formats::JsonArrayStreamFormat;
#[cfg(feature = "json")]
pub use json_formats::JsonNewLineStreamFormat;

#[cfg(feature = "csv")]
mod csv_format;
#[cfg(feature = "csv")]
pub use csv_format::CsvStreamFormat;

#[cfg(test)]
mod test_client;
