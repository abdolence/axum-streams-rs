use crate::stream_format::StreamingFormat;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::HeaderMap;
use serde::Serialize;

pub struct JsonArrayStreamFormat;

impl JsonArrayStreamFormat {
    pub fn new() -> Self {
        Self {}
    }
}

impl<T> StreamingFormat<T> for JsonArrayStreamFormat
where
    T: Serialize + Send + Sync + 'static,
{
    fn bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.enumerate().map(|(index, obj)| {
                let mut output = vec![];
                serde_json::to_vec::<T>(&obj)
                    .map(|obj_vec| {
                        if index != 0 {
                            output.extend(JSON_ARRAY_SEP_BYTES)
                        }
                        output.extend(obj_vec);
                        axum::body::Bytes::from(output)
                    })
                    .map_err(axum::Error::new)
            })
        });

        let prepend_stream: BoxStream<Result<axum::body::Bytes, axum::Error>> =
            Box::pin(futures_util::stream::once(futures_util::future::ready(
                Ok::<_, axum::Error>(axum::body::Bytes::from(JSON_ARRAY_BEGIN_BYTES)),
            )));

        let append_stream: BoxStream<Result<axum::body::Bytes, axum::Error>> =
            Box::pin(futures_util::stream::once(futures_util::future::ready(
                Ok::<_, axum::Error>(axum::body::Bytes::from(JSON_ARRAY_END_BYTES)),
            )));

        Box::pin(prepend_stream.chain(stream_bytes.chain(append_stream)))
    }

    fn http_response_trailers(&self) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        Some(header_map)
    }
}

pub struct JsonNewLineStreamFormat;

impl JsonNewLineStreamFormat {
    pub fn new() -> Self {
        Self {}
    }
}

impl<T> StreamingFormat<T> for JsonNewLineStreamFormat
where
    T: Serialize + Send + Sync + 'static,
{
    fn bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.enumerate().map(|(index, obj)| {
                let mut output = vec![];
                serde_json::to_vec::<T>(&obj)
                    .map(|obj_vec| {
                        if index != 0 {
                            output.extend(JSON_NL_SEP_BYTES)
                        }
                        output.extend(obj_vec);
                        axum::body::Bytes::from(output)
                    })
                    .map_err(axum::Error::new)
            })
        });

        Box::pin(stream_bytes)
    }

    fn http_response_trailers(&self) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            "application/jsonstream".parse().unwrap(),
        );
        Some(header_map)
    }
}

const JSON_ARRAY_BEGIN_BYTES: &[u8] = "[".as_bytes();
const JSON_ARRAY_END_BYTES: &[u8] = "]".as_bytes();
const JSON_ARRAY_SEP_BYTES: &[u8] = ",".as_bytes();

const JSON_NL_SEP_BYTES: &[u8] = "\n".as_bytes();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyWithFormat;
    use axum::{routing::*, Router};
    use futures_util::stream;

    #[tokio::test]
    async fn serialize_json_array_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async { StreamBodyWithFormat::new(JsonArrayStreamFormat::new(), test_stream) }),
        );

        let client = TestClient::new(app);

        let expected_json = serde_json::to_string(&test_stream_vec).unwrap();
        let res = client.get("/").send().await.unwrap();
        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }

    #[tokio::test]
    async fn serialize_json_nl_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo: "bar".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyWithFormat::new(JsonNewLineStreamFormat::new(), test_stream)
            }),
        );

        let client = TestClient::new(app);

        let expected_json = test_stream_vec
            .iter()
            .map(|item| serde_json::to_string(item).unwrap())
            .collect::<Vec<String>>()
            .join("\n");

        let res = client.get("/").send().await.unwrap();
        let body = res.text().await.unwrap();

        assert_eq!(body, expected_json);
    }
}
