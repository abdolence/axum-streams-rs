use crate::StreamingFormat;
use arrow::array::RecordBatch;
use arrow::datatypes::{Schema, SchemaRef};
use arrow::ipc::writer::{IpcWriteOptions, StreamWriter};
use bytes::{BufMut, BytesMut};
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;
use http_body::Frame;
use std::sync::Arc;

pub struct ArrowRecordBatchStreamFormat {
    schema: SchemaRef,
    options: IpcWriteOptions,
}

impl ArrowRecordBatchStreamFormat {
    pub fn new(schema: Arc<Schema>) -> Self {
        Self::with_options(schema, IpcWriteOptions::default())
    }

    pub fn with_options(schema: Arc<Schema>, options: IpcWriteOptions) -> Self {
        Self {
            schema: schema.clone(),
            options: options.clone(),
        }
    }
}

impl StreamingFormat<RecordBatch> for ArrowRecordBatchStreamFormat {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, RecordBatch>,
    ) -> BoxStream<'b, Result<Frame<axum::body::Bytes>, axum::Error>> {
        let schema = self.schema.clone();
        let options = self.options.clone();

        let stream_bytes: BoxStream<Result<Frame<axum::body::Bytes>, axum::Error>> = Box::pin({
            stream.map(move |batch| {
                let buf = BytesMut::new().writer();
                let mut writer = StreamWriter::try_new_with_options(buf, &schema, options.clone())
                    .map_err(axum::Error::new)?;
                writer.write(&batch).map_err(axum::Error::new)?;
                writer.finish().map_err(axum::Error::new)?;
                writer
                    .into_inner()
                    .map_err(axum::Error::new)
                    .map(|buf| buf.into_inner().freeze())
                    .map(axum::body::Bytes::from)
                    .map(Frame::data)
            })
        });

        Box::pin(stream_bytes)
    }

    fn http_response_trailers(&self) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("application/vnd.apache.arrow.stream"),
        );
        Some(header_map)
    }
}

impl<'a> crate::StreamBodyAs<'a> {
    pub fn arrow<S>(schema: SchemaRef, stream: S) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(ArrowRecordBatchStreamFormat::new(schema), stream)
    }

    pub fn arrow_with_options<S>(schema: SchemaRef, stream: S, options: IpcWriteOptions) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(
            ArrowRecordBatchStreamFormat::with_options(schema, options),
            stream,
        )
    }
}

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
                    ArrowRecordBatchStreamFormat::new(app_schema.clone()),
                    test_stream,
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_proto_buf: Vec<u8> = create_test_batch(schema.clone())
            .iter()
            .flat_map(|batch| {
                let mut writer = StreamWriter::try_new(Vec::new(), &schema).expect("writer failed");
                writer.write(&batch).expect("write failed");
                writer.finish().expect("writer failed");
                writer.into_inner().expect("writer failed")
            })
            .collect();

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/vnd.apache.arrow.stream")
        );
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body, expected_proto_buf);
    }
}
