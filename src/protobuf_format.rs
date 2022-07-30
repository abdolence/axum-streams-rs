use crate::stream_format::StreamingFormat;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::HeaderMap;

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
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        fn write_protobuf_record<T>(obj: T) -> Result<Vec<u8>, axum::Error>
        where
            T: prost::Message,
        {
            let obj_vec = obj.encode_to_vec();
            let mut frame_vec = Vec::new();
            prost::encoding::uint64::encode(1, &(obj_vec.len() as u64), &mut frame_vec);
            frame_vec.extend(obj_vec);

            Ok(frame_vec)
        }

        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.map(move |obj| {
                let write_protobuf_res = write_protobuf_record(obj);
                write_protobuf_res.map(axum::body::Bytes::from)
            })
        });

        Box::pin(stream_bytes)
    }

    fn http_response_trailers(&self) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            "application/x-protobuf-stream".parse().unwrap(),
        );
        Some(header_map)
    }
}

impl<'a> crate::StreamBodyWith<'a> {
    pub fn protobuf<T>(stream: BoxStream<'a, T>) -> Self
    where
        T: prost::Message + Send + Sync + 'static,
    {
        Self::new(ProtobufStreamFormat::new(), stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyWith;
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
            get(|| async { StreamBodyWith::new(ProtobufStreamFormat::new(), test_stream) }),
        );

        let client = TestClient::new(app);

        let expected_proto_buf: Vec<u8> = test_stream_vec
            .iter()
            .flat_map(|obj| {
                let obj_vec = obj.encode_to_vec();
                let mut frame_vec = Vec::new();
                prost::encoding::uint64::encode(1, &(obj_vec.len() as u64), &mut frame_vec);
                frame_vec.extend(obj_vec);
                frame_vec
            })
            .collect();

        let res = client.get("/").send().await.unwrap();
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body, expected_proto_buf);
    }
}
