use crate::stream_body_as::StreamBodyAsOptions;
use crate::stream_format::StreamingFormat;
use crate::StreamBodyAs;
use futures::stream::BoxStream;
use futures::Stream;
use futures::StreamExt;
use http::HeaderMap;
use serde::Serialize;

pub struct CsvStreamFormat {
    has_headers: bool,
    delimiter: u8,
    flexible: bool,
    quote_style: csv::QuoteStyle,
    quote: u8,
    double_quote: bool,
    escape: u8,
    terminator: csv::Terminator,
}

impl Default for CsvStreamFormat {
    fn default() -> Self {
        Self {
            has_headers: true,
            delimiter: b',',
            flexible: false,
            quote_style: csv::QuoteStyle::Necessary,
            quote: b'"',
            double_quote: true,
            escape: b'\\',
            terminator: csv::Terminator::Any(b'\n'),
        }
    }
}

impl CsvStreamFormat {
    pub fn new(has_headers: bool, delimiter: u8) -> Self {
        Self {
            has_headers,
            delimiter,
            ..Default::default()
        }
    }

    /// Sets whether to use flexible serialize.
    pub fn with_flexible(mut self, flexible: bool) -> Self {
        self.flexible = flexible;
        self
    }

    /// Sets the quote style to use.
    pub fn with_quote_style(mut self, quote_style: csv::QuoteStyle) -> Self {
        self.quote_style = quote_style;
        self
    }

    /// Sets the quote character to use.
    pub fn with_quote(mut self, quote: u8) -> Self {
        self.quote = quote;
        self
    }

    /// Sets whether to double quote.
    pub fn with_double_quote(mut self, double_quote: bool) -> Self {
        self.double_quote = double_quote;
        self
    }

    /// Sets the escape character to use.
    pub fn with_escape(mut self, escape: u8) -> Self {
        self.escape = escape;
        self
    }

    /// Sets the line terminator to use.
    pub fn with_terminator(mut self, terminator: csv::Terminator) -> Self {
        self.terminator = terminator;
        self
    }

    /// Set the field delimiter to use.
    pub fn with_delimiter(mut self, delimiter: u8) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Set whether to write headers.
    pub fn with_has_headers(mut self, has_headers: bool) -> Self {
        self.has_headers = has_headers;
        self
    }
}

impl<T> StreamingFormat<T> for CsvStreamFormat
where
    T: Serialize + Send + Sync + 'static,
{
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, Result<T, axum::Error>>,
        _: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>> {
        let stream_with_header = self.has_headers;
        let stream_delimiter = self.delimiter;
        let stream_flexible = self.flexible;
        let stream_quote_style = self.quote_style;
        let stream_quote = self.quote;
        let stream_double_quote = self.double_quote;
        let stream_escape = self.escape;
        let terminator = self.terminator;

        Box::pin({
            stream
                .enumerate()
                .map(move |(index, obj_res)| match obj_res {
                    Err(e) => Err(e),
                    Ok(obj) => {
                        let mut writer = csv::WriterBuilder::new()
                            .has_headers(index == 0 && stream_with_header)
                            .delimiter(stream_delimiter)
                            .flexible(stream_flexible)
                            .quote_style(stream_quote_style)
                            .quote(stream_quote)
                            .double_quote(stream_double_quote)
                            .escape(stream_escape)
                            .terminator(terminator)
                            .from_writer(vec![]);

                        writer.serialize(obj).map_err(axum::Error::new)?;
                        writer.flush().map_err(axum::Error::new)?;
                        writer
                            .into_inner()
                            .map_err(axum::Error::new)
                            .map(axum::body::Bytes::from)
                    }
                })
        })
    }

    fn http_response_headers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            options
                .content_type
                .clone()
                .unwrap_or_else(|| http::header::HeaderValue::from_static("text/csv")),
        );
        Some(header_map)
    }
}

impl<'a> StreamBodyAs<'a> {
    pub fn csv<S, T>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        Self::new(
            CsvStreamFormat::new(false, b','),
            stream.map(Ok::<T, axum::Error>),
        )
    }

    pub fn csv_with_errors<S, T, E>(stream: S) -> Self
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error> + 'static,
    {
        Self::new(CsvStreamFormat::new(false, b','), stream)
    }
}

impl StreamBodyAsOptions {
    pub fn csv<'a, S, T>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = T> + 'a + Send,
    {
        StreamBodyAs::with_options(
            CsvStreamFormat::new(false, b','),
            stream.map(Ok::<T, axum::Error>),
            self,
        )
    }

    pub fn csv_with_errors<'a, S, T, E>(self, stream: S) -> StreamBodyAs<'a>
    where
        T: Serialize + Send + Sync + 'static,
        S: Stream<Item = Result<T, E>> + 'a + Send,
        E: Into<axum::Error> + 'static,
    {
        StreamBodyAs::with_options(CsvStreamFormat::new(false, b','), stream, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use crate::StreamBodyAs;
    use axum::{routing::*, Router};
    use futures::stream;
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
            get(|| async {
                StreamBodyAs::new(
                    CsvStreamFormat::new(false, b'.').with_delimiter(b','),
                    test_stream.map(Ok::<_, axum::Error>),
                )
            }),
        );

        let client = TestClient::new(app).await;

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
