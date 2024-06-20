#![allow(unused_parens, clippy::new_without_default)]
#![forbid(unsafe_code)]

//! # axum HTTP streaming body support for different formats:
//! - JSON array stream format
//! - JSON Lines (NL/NewLines) format
//! - CSV stream format
//! - Protobuf len-prefixed stream format
//! - Arrow IPC stream format
//! - Text stream format
//!
//! [JSON Streaming](https://en.wikipedia.org/wiki/JSON_streaming) is a term referring to streaming a
//! stream of element as independent JSON objects as a continuous HTTP request or response.
//!
//! This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
//! and want to avoid huge memory allocations to store on the server side.
//!
//! # Example
//!
//! ```rust
//! use axum::{
//!     Router,
//!     routing::get,
//!     http::{StatusCode, header::CONTENT_TYPE},
//!     response::{Response, IntoResponse},
//! };
//! use futures::Stream;
//! use axum_streams::*;
//! use serde::Serialize;
//!
//! #[derive(Debug, Clone, Serialize)]
//! struct MyTestStructure {
//!     some_test_field: String
//! }
//!
//! // Your possibly stream of objects
//! fn my_source_stream() -> impl Stream<Item=MyTestStructure> {
//!     // Simulating a stream with a plain vector and throttling to show how it works
//!     use tokio_stream::StreamExt;
//!     futures::stream::iter(vec![
//!         MyTestStructure {
//!             some_test_field: "test1".to_string()
//!         }; 1000
//!     ]).throttle(std::time::Duration::from_millis(50))
//! }
//!
//! // Route implementation:
//! async fn test_json_array_stream() -> impl IntoResponse {
//!     StreamBodyAs::json_array(my_source_stream())
//! }
//! async fn test_json_nl_stream() -> impl IntoResponse {
//!     StreamBodyAs::json_nl(my_source_stream())
//! }
//!
//! async fn test_csv_stream() -> impl IntoResponse {
//!     StreamBodyAs::csv(my_source_stream())
//!     // Which is the same as:
//!     // StreamBodyWith::new(CsvStreamFormat::new(
//!     //    true, // with_header
//!     //    b',' // CSV delimiter
//!     //), my_source_stream())
//! }
//!
//! ```
//! ## Need client support?
//! There is the same functionality for:
//! - [reqwest-streams](https://github.com/abdolence/reqwest-streams-rs).
//!

#![allow(clippy::new_without_default, clippy::needless_lifetimes)]

mod stream_format;
pub use stream_format::*;

mod stream_body_as;
pub use self::stream_body_as::HttpHeaderValue;
pub use self::stream_body_as::StreamBodyAs;
pub use self::stream_body_as::StreamBodyAsOptions;

mod envelope;
pub use envelope::*;

#[cfg(feature = "json")]
mod json_formats;
#[cfg(feature = "json")]
pub use json_formats::JsonArrayStreamFormat;
#[cfg(feature = "json")]
pub use json_formats::JsonNewLineStreamFormat;

#[cfg(feature = "csv")]
mod csv_format;
#[cfg(feature = "csv")]
pub use csv::{QuoteStyle, Terminator};
#[cfg(feature = "csv")]
pub use csv_format::CsvStreamFormat;

#[cfg(feature = "text")]
mod text_format;
#[cfg(feature = "text")]
pub use text_format::TextStreamFormat;

#[cfg(feature = "protobuf")]
mod protobuf_format;
#[cfg(feature = "protobuf")]
pub use protobuf_format::ProtobufStreamFormat;

#[cfg(feature = "arrow")]
mod arrow_format;
#[cfg(feature = "arrow")]
pub use arrow_format::ArrowRecordBatchIpcStreamFormat;

#[cfg(test)]
mod test_client;
