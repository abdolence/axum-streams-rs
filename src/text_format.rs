use crate::stream_body_as::StreamBodyAsOptions;
use crate::stream_format::StreamingFormat;
use crate::StreamBodyAs;
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;

pub struct TextStreamFormat;

impl TextStreamFormat {
    pub fn new() -> Self {
        Self {}
    }
}

impl StreamingFormat<String> for TextStreamFormat {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<String, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        fn write_text_record(obj: String) -> Result<Vec<u8>, axum::Error> {
            let obj_vec = obj.as_bytes().to_vec();
            Ok(obj_vec)
        }

        Box::pin(stream.map(move |obj_res| match obj_res {
            Err(e) => Err(e),
            Ok(obj) => write_text_record(obj).map(|data| data.into()),
        }))
    }

    fn http_response_headers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            options.content_type.clone().unwrap_or_else(|| {
                http::header::HeaderValue::from_static("text/plain; charset=utf-8")
            }),
        );
        Some(header_map)
    }
}

impl<'a> StreamBodyAs<'a> {
    pub fn text<S>(stream: S) -> Self
    where
        S: Stream<Item = String> + 'a + Send,
    {
        Self::new(
            TextStreamFormat::new(),
            stream.map(Ok::<String, axum::Error>),
        )
    }

    pub fn text_with_errors<S, E>(stream: S) -> Self
    where
        S: Stream<Item = Result<String, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        Self::new(TextStreamFormat::new(), stream)
    }
}

impl StreamBodyAsOptions {
    pub fn text<'a, S>(self, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = String> + 'a + Send,
    {
        StreamBodyAs::with_options(
            TextStreamFormat::new(),
            stream.map(Ok::<String, axum::Error>),
            self,
        )
    }

    pub fn text_with_errors<'a, S, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        S: Stream<Item = Result<String, E>> + 'a + Send,
        E: Into<axum::Error>,
    {
        StreamBodyAs::with_options(TextStreamFormat::new(), stream, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures::stream;

    #[tokio::test]
    async fn serialize_text_stream_format() {
        #[derive(Clone, prost::Message)]
        struct TestOutputStructure {
            #[prost(string, tag = "1")]
            foo1: String,
            #[prost(string, tag = "2")]
            foo2: String,
        }

        let test_stream_vec = vec![
            String::from("bar1"),
            String::from("bar2"),
            String::from("bar3"),
            String::from("bar4"),
            String::from("bar5"),
            String::from("bar6"),
            String::from("bar7"),
            String::from("bar8"),
            String::from("bar9"),
        ];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async {
                StreamBodyAs::new(
                    TextStreamFormat::new(),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

        let expected_text_buf: Vec<u8> = test_stream_vec
            .iter()
            .flat_map(|obj| {
                let obj_vec = obj.as_bytes().to_vec();
                obj_vec
            })
            .collect();

        let res = client.get("/").send().await.unwrap();
        let body = res.bytes().await.unwrap().to_vec();

        assert_eq!(body, expected_text_buf);
    }
}
