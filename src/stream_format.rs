use futures::stream::BoxStream;
use http::HeaderMap;

pub trait StreamingFormat<T> {
    fn to_bytes_stream<'a, 'b>(
        &'a self,
        stream: BoxStream<'b, T>,
    ) -> BoxStream<'b, Result<http_body::Frame<axum::body::Bytes>, axum::Error>>;

    fn http_response_trailers(&self) -> Option<HeaderMap>;
}
