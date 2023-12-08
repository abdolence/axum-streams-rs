use crate::stream_format::StreamingFormat;
use futures::Stream;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::HeaderMap;
use http_body::Frame;

pub struct ProtobufStreamFormat;

impl ProtobufStreamFormat {
    pub fn new() -> Self {
        Self {}
    }
}

impl<T> StreamingFormat<T> for ProtobufStreamFormat
where
    T: prost::Message + Send + Sync + 'static,
{
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<Frame<axum::body::Bytes>, axum::Error>> {
        fn write_protobuf_record<T>(obj: T) -> Result<Vec<u8>, axum::Error>
        where
            T: prost::Message,
        {
            let obj_vec = obj.encode_to_vec();
            let mut frame_vec = Vec::new();
            let obj_len = (obj_vec.len() as u64);
            prost::encoding::encode_varint(obj_len, &mut frame_vec);
            frame_vec.extend(obj_vec);

            Ok(frame_vec)
        }

        let stream_bytes: BoxStream<Result<Frame<axum::body::Bytes>, axum::Error>> = Box::pin({
            stream.map(move |obj| {
                let write_protobuf_res = write_protobuf_record(obj);
                write_protobuf_res
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
            http::header::HeaderValue::from_static("application/x-protobuf-stream"),
        );
        Some(header_map)
    }
}

impl<'a> crate::StreamBodyAs<'a> {
    pub fn protobuf<S, T>(stream: S) -> Self
    where
        T: prost::Message + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(ProtobufStreamFormat::new(), stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures_util::stream;
    use prost::Message;

    #[tokio::test]
    async fn serialize_protobuf_stream_format() {
        #[derive(Clone, prost::Message)]
        struct TestOutputStructure {
            #[prost(string, tag = "1")]
            foo1: String,
            #[prost(string, tag = "2")]
            foo2: String,
        }

        let test_stream_vec = vec![
            TestOutputStructure {
                foo1: "bar1".to_string(),
                foo2: "bar2".to_string()
            };
            7
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async { StreamBodyAs::new(ProtobufStreamFormat::new(), test_stream) }),
        );

        let client = TestClient::new(app);

        let expected_proto_buf: Vec<u8> = test_stream_vec
            .iter()
            .flat_map(|obj| {
                let obj_vec = obj.encode_to_vec();
                let mut frame_vec = Vec::new();
                let obj_len = (obj_vec.len() as u64);
                prost::encoding::encode_varint(obj_len, &mut frame_vec);
                frame_vec.extend(obj_vec);
                frame_vec
            })
            .collect();

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("application/x-protobuf-stream")
        );
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body, expected_proto_buf);
    }
}
