use crate::StreamBodyAsOptions;
use futures::stream::BoxStream;
use http::HeaderMap;

pub trait StreamingFormat<T> {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
        options: &'a StreamBodyAsOptions,
    ) -> BoxStream<'b, Result<axum::body::Bytes, axum::Error>>;

    fn http_response_trailers(&self, options: &StreamBodyAsOptions) -> Option<HeaderMap>;
}
