use crate::stream_format::StreamingFormat;
use futures::Stream;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use http::HeaderMap;
use serde::Serialize;

pub struct CsvStreamFormat {
    with_header: bool,
    delimiter: u8,
}

impl CsvStreamFormat {
    pub fn new(with_header: bool, delimiter: u8) -> Self {
        Self {
            with_header,
            delimiter,
        }
    }
}

impl<T> StreamingFormat<T> for CsvStreamFormat
where
    T: Serialize + Send + Sync + 'static,
{
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        fn write_csv_record<T>(obj: T, header: bool, delimiter: u8) -> Result<Vec<u8>, axum::Error>
        where
            T: Serialize,
        {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(header)
                .delimiter(delimiter)
                .from_writer(vec![]);
            writer.serialize(obj).map_err(axum::Error::new)?;
            writer.flush().map_err(axum::Error::new)?;
            writer.into_inner().map_err(axum::Error::new)
        }

        let stream_with_header = self.with_header;
        let stream_delimiter = self.delimiter;
        let stream_bytes: BoxStream<Result<axum::body::Bytes, axum::Error>> = Box::pin({
            stream.enumerate().map(move |(index, obj)| {
                let write_csv_res = if index == 0 {
                    write_csv_record(obj, stream_with_header, stream_delimiter)
                } else {
                    write_csv_record(obj, false, stream_delimiter)
                };

                write_csv_res.map(axum::body::Bytes::from)
            })
        });

        Box::pin(stream_bytes)
    }

    fn http_response_trailers(&self) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(http::header::CONTENT_TYPE, "text/csv".parse().unwrap());
        Some(header_map)
    }
}

impl<'a> crate::StreamBodyAs<'a> {
    pub fn csv<S, T>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(CsvStreamFormat::new(false, b','), stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures_util::stream;
    use std::ops::Add;

    #[tokio::test]
    async fn serialize_csv_stream_format() {
        #[derive(Debug, Clone, Serialize)]
        struct TestOutputStructure {
            foo1: String,
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
            get(|| async { StreamBodyAs::new(CsvStreamFormat::new(false, b','), test_stream) }),
        );

        let client = TestClient::new(app);

        let expected_csv = test_stream_vec
            .iter()
            .map(|item| format!("{},{}", item.foo1, item.foo2))
            .collect::<Vec<String>>()
            .join("\n")
            .add("\n");

        let res = client.get("/").send().await.unwrap();
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok()),
            Some("text/csv")
        );
        let body = res.text().await.unwrap();

        assert_eq!(body, expected_csv);
    }
}
