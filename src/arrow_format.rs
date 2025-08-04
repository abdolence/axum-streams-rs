use crate::stream_body_as::StreamBodyAsOptions;
use crate::{StreamBodyAs, StreamingFormat};
use arrow::array::RecordBatch;
use arrow::datatypes::{Schema, SchemaRef};
use arrow::error::ArrowError;
use arrow::ipc::writer::{write_message, DictionaryTracker, IpcDataGenerator, IpcWriteOptions};
use bytes::{BufMut, BytesMut};
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;
use std::io::Write;
use std::sync::Arc;

pub struct ArrowRecordBatchIpcStreamFormat {
    schema: Option<SchemaRef>,
    options: IpcWriteOptions,
}

impl ArrowRecordBatchIpcStreamFormat {
    pub fn new(schema: Option<SchemaRef>) -> Self {
        Self::with_options(schema, IpcWriteOptions::default())
    }

    pub fn with_options(schema: Option<SchemaRef>, options: IpcWriteOptions) -> Self {
        Self {
            schema: schema.clone(),
            options: options.clone(),
        }
    }
}

impl StreamingFormat<RecordBatch> for ArrowRecordBatchIpcStreamFormat {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<RecordBatch, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        fn write_batch(
            ipc_data_gen: &mut IpcDataGenerator,
            dictionary_tracker: &mut DictionaryTracker,
            write_options: &IpcWriteOptions,
            batch: &RecordBatch,
            prepend_schema: Option<Arc<Schema>>,
        ) -> Result<axum::body::Bytes, ArrowError> {
            let mut writer = BytesMut::new().writer();

            if let Some(prepend_schema) = prepend_schema {
                let encoded_message = ipc_data_gen.schema_to_bytes_with_dictionary_tracker(
                    &prepend_schema,
                    dictionary_tracker,
                    write_options,
                );
                write_message(&mut writer, encoded_message, write_options)?;
            }

            let (encoded_dictionaries, encoded_message) =
                ipc_data_gen.encoded_batch(batch, dictionary_tracker, write_options)?;

            for encoded_dictionary in encoded_dictionaries {
                write_message(&mut writer, encoded_dictionary, write_options)?;
            }

            write_message(&mut writer, encoded_message, write_options)?;
            writer.flush()?;
            Ok(writer.into_inner().freeze())
        }

        fn write_continuation() -> Result<axum::body::Bytes, ArrowError> {
            let mut writer = BytesMut::with_capacity(8).writer();
            const CONTINUATION_MARKER: [u8; 4] = [0xff; 4];
            const TOTAL_LEN: [u8; 4] = [0; 4];
            writer.write_all(&CONTINUATION_MARKER)?;
            writer.write_all(&TOTAL_LEN)?;
            writer.flush()?;
            Ok(writer.into_inner().freeze())
        }

        let batch_maybe_schema = self.schema.clone();
        let batch_options = self.options.clone();

        let ipc_data_gen = IpcDataGenerator::default();
        let dictionary_tracker: DictionaryTracker = DictionaryTracker::new(false);

        let batch_stream = Box::pin({
            stream.scan(
                (ipc_data_gen, dictionary_tracker, 0),
                move |(ipc_data_gen, dictionary_tracker, idx), batch_res| match batch_res {
                    Err(e) => futures::future::ready(Some(Err(e))),
                    Ok(batch) => futures::future::ready({
                        let prepend_schema = if *idx == 0 {
                            batch_maybe_schema
                                .as_ref()
                                .map(|batch_schema| batch_schema.clone())
                        } else {
                            None
                        };
                        *idx += 1;
                        let bytes = write_batch(
                            ipc_data_gen,
                            dictionary_tracker,
                            &batch_options,
                            &batch,
                            prepend_schema,
                        )
                        .map_err(axum::Error::new);
                        Some(bytes)
                    }),
                },
            )
        });

        let append_stream: BoxStream<Result<axum::body::Bytes, axum::Error>> =
            Box::pin(futures::stream::once(futures::future::ready({
                write_continuation().map_err(axum::Error::new)
            })));

        Box::pin(batch_stream.chain(append_stream))
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
}

impl<'a> crate::StreamBodyAs<'a> {
    pub fn arrow_ipc<S>(schema: Option<SchemaRef>, stream: S) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream.map(Ok::<RecordBatch, axum::Error>),
        )
    }

    pub fn arrow_ipc_with_errors<S, E>(schema: Option<SchemaRef>, stream: S) -> Self
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(ArrowRecordBatchIpcStreamFormat::new(schema), stream)
    }

    pub fn arrow_ipc_with_options<S>(
        schema: Option<SchemaRef>,
        stream: S,
        options: IpcWriteOptions,
    ) -> Self
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        Self::new(
            ArrowRecordBatchIpcStreamFormat::with_options(schema, options),
            stream.map(Ok::<RecordBatch, axum::Error>),
        )
    }

    pub fn arrow_ipc_with_options_errors<S, E>(
        schema: Option<SchemaRef>,
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
    pub fn arrow_ipc<'a, S>(self, schema: Option<SchemaRef>, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = RecordBatch> + 'a + Send,
    {
        StreamBodyAs::with_options(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream.map(Ok::<RecordBatch, axum::Error>),
            self,
        )
    }

    pub fn arrow_ipc_with_errors<'a, S, E>(
        self,
        schema: Option<SchemaRef>,
        stream: S,
    ) -> StreamBodyAs<'a>
    where
        S: Stream<Item = Result<RecordBatch, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(ArrowRecordBatchIpcStreamFormat::new(schema), stream, self)
    }

    pub fn arrow_ipc_with_options<'a, S>(
        self,
        schema: Option<SchemaRef>,
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
        schema: Option<SchemaRef>,
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
                    ArrowRecordBatchIpcStreamFormat::new(Some(app_schema.clone())),
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

    #[tokio::test]
    async fn serialize_arrow_stream_format_without_schema() {
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

        let app = Router::new().route(
            "/",
            get(|| async move {
                StreamBodyAs::new(
                    ArrowRecordBatchIpcStreamFormat::new(None),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/vnd.apache.arrow.stream")
        );
        let body = res.bytes().await.unwrap().to_vec();

        assert!(body.len() > 0);
    }
}
