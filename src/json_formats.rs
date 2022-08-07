use crate::stream_format::StreamingFormat;
use bytes::{BufMut, BytesMut};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::HeaderMap;
use serde::Serialize;
use std::io::Write;

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
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.enumerate().map(|(index, obj)| {
                let mut buf = BytesMut::new().writer();
                if index != 0 {
                    buf.write_all(JSON_ARRAY_SEP_BYTES).unwrap();
                }
                match serde_json::to_writer(&mut buf, &obj) {
                    Ok(_) => Ok(buf.into_inner().freeze()),
                    Err(e) => Err(axum::Error::new(e)),
                }
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
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.enumerate().map(|(index, obj)| {
                let mut buf = BytesMut::new().writer();
                if index != 0 {
                    buf.write_all(JSON_NL_SEP_BYTES).unwrap();
                }
                match serde_json::to_writer(&mut buf, &obj) {
                    Ok(_) => Ok(buf.into_inner().freeze()),
                    Err(e) => Err(axum::Error::new(e)),
                }
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

impl<'a> crate::StreamBodyAs<'a> {
    pub fn json_array<T>(stream: BoxStream<'a, T>) -> Self
    where
        T: Serialize + Send + Sync + 'static,
    {
        Self::new(JsonArrayStreamFormat::new(), stream)
    }

    pub fn json_nl<T>(stream: BoxStream<'a, T>) -> Self
    where
        T: Serialize + Send + Sync + 'static,
    {
        Self::new(JsonNewLineStreamFormat::new(), stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
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
            get(|| async { StreamBodyAs::new(JsonArrayStreamFormat::new(), test_stream) }),
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
            get(|| async { StreamBodyAs::new(JsonNewLineStreamFormat::new(), test_stream) }),
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
