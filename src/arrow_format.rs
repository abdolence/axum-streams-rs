//! Apache Arrow IPC responses.
//!
//! The format itself lives in [`http_streams_core`] and is re-exported here unchanged, so that
//! this crate and `reqwest-streams` produce byte-identical bodies from one implementation.
//! What remains here is the [`StreamingFormat`] shim — that trait names [`axum::Error`], which
//! core cannot — and the response headers, which are an HTTP concern rather than a framing one.

use crate::stream_body_as::StreamBodyAsOptions;
use crate::stream_encoding::encode_items;
use crate::stream_format::StreamingFormat;
use crate::StreamBodyAs;
use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use arrow::ipc::writer::IpcWriteOptions;
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;

pub use http_streams_core::ArrowRecordBatchIpcStreamFormat;

impl StreamingFormat<RecordBatch> for ArrowRecordBatchIpcStreamFormat {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<RecordBatch, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        encode_items(self, stream)
    }

    fn http_response_headers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            options.content_type.clone().unwrap_or_else(|| {
                http::header::HeaderValue::from_static("application/vnd.apache.arrow.stream")
            }),
        );
        Some(header_map)
    }

    fn format_name(&self) -> Option<&str> {
        Some("arrow")
    }
}
impl<'a> crate::StreamBodyAs<'a> {
    pub fn arrow_ipc<S>(schema: SchemaRef, stream: S) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream.map(Ok::<RecordBatch, axum::Error>),
        )
    }

    pub fn arrow_ipc_with_errors<S, E>(schema: SchemaRef, stream: S) -> Self
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(ArrowRecordBatchIpcStreamFormat::new(schema), stream)
    }

    pub fn arrow_ipc_with_options<S>(schema: SchemaRef, stream: S, options: IpcWriteOptions) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(
            ArrowRecordBatchIpcStreamFormat::with_options(schema, options),
            stream.map(Ok::<RecordBatch, axum::Error>),
        )
    }

    pub fn arrow_ipc_with_options_errors<S, E>(
        schema: SchemaRef,
        stream: S,
        options: IpcWriteOptions,
    ) -> Self
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(
            ArrowRecordBatchIpcStreamFormat::with_options(schema, options),
            stream,
        )
    }
}

impl StreamBodyAsOptions {
    pub fn arrow_ipc<'a, S>(self, schema: SchemaRef, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        StreamBodyAs::with_options(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream.map(Ok::<RecordBatch, axum::Error>),
            self,
        )
    }

    pub fn arrow_ipc_with_errors<'a, S, E>(self, schema: SchemaRef, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(ArrowRecordBatchIpcStreamFormat::new(schema), stream, self)
    }

    pub fn arrow_ipc_with_options<'a, S>(
        self,
        schema: SchemaRef,
        stream: S,
        options: IpcWriteOptions,
    ) -> StreamBodyAs<'a>
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        StreamBodyAs::with_options(
            ArrowRecordBatchIpcStreamFormat::with_options(schema, options),
            stream.map(Ok::<RecordBatch, axum::Error>),
            self,
        )
    }

    pub fn arrow_ipc_with_options_errors<'a, S, E>(
        self,
        schema: SchemaRef,
        stream: S,
        options: IpcWriteOptions,
    ) -> StreamBodyAs<'a>
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(
            ArrowRecordBatchIpcStreamFormat::with_options(schema, options),
            stream,
            self,
        )
    }
}

/// An Arrow IPC request body, decoded into a stream of record batches.
///
/// No schema is needed: an Arrow IPC stream carries its own.
pub type ArrowIpcStreamFrom =
    crate::StreamBodyFrom<ArrowRecordBatchIpcStreamFormat, arrow::array::RecordBatch>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use arrow::array::*;
    use arrow::datatypes::*;
    use axum::{routing::*, Router};
    use futures::stream;
    use std::sync::Arc;

    #[tokio::test]
    async fn serialize_arrow_stream_format() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("city", DataType::Utf8, false),
            Field::new("lat", DataType::Float64, false),
            Field::new("lng", DataType::Float64, false),
        ]));

        fn create_test_batch(schema_ref: SchemaRef) -> Vec<RecordBatch> {
            let vec_schema = schema_ref.clone();
            (0i64..10i64)
                .map(move |idx| {
                    RecordBatch::try_new(
                        vec_schema.clone(),
                        vec![
                            Arc::new(Int64Array::from(vec![idx, idx * 2, idx * 3])),
                            Arc::new(StringArray::from(vec!["New York", "London", "Gothenburg"])),
                            Arc::new(Float64Array::from(vec![40.7128, 51.5074, 57.7089])),
                            Arc::new(Float64Array::from(vec![-74.0060, -0.1278, 11.9746])),
                        ],
                    )
                    .unwrap()
                })
                .collect()
        }

        let test_stream = Box::pin(stream::iter(create_test_batch(schema.clone())));

        let app_schema = schema.clone();

        let app = Router::new().route(
            "/",
            get(|| async move {
                StreamBodyAs::new(
                    ArrowRecordBatchIpcStreamFormat::new(app_schema.clone()),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let mut writer =
            arrow::ipc::writer::StreamWriter::try_new(Vec::new(), &schema).expect("writer failed");
        for batch in create_test_batch(schema.clone()) {
            writer.write(&batch).expect("write failed");
        }
        writer.finish().expect("writer failed");
        let expected_buf = writer.into_inner().expect("writer failed");

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/vnd.apache.arrow.stream")
        );
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body.len(), expected_buf.len());
        assert_eq!(body, expected_buf);
    }
}
