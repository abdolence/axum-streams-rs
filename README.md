[![Cargo](https://img.shields.io/crates/v/axum-streams.svg)](https://crates.io/crates/axum-streams)
![tests and formatting](https://github.com/abdolence/axum-streams-rs/workflows/tests%20&amp;%20formatting/badge.svg)
![security audit](https://github.com/abdolence/axum-streams-rs/workflows/security%20audit/badge.svg)

# axum streams for Rust

Library provides HTTP response streaming support for [axum web framework](https://github.com/tokio-rs/axum):
- JSON array stream format
  - Support for simple envelopes structures when you need to include your array inside some object (only for first level) 
- JSON lines stream format
- CSV stream
- Protobuf len-prefixed stream format
- Apache Arrow IPC stream format
- Text stream

This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
and want to avoid huge memory allocation.

## Quick start

Cargo.toml:
```toml
[dependencies]
axum-streams = { version = "0.29", features=["json", "csv", "protobuf", "text", "arrow"] }
```

## Compatibility matrix

| axum | axum-streams |
|------|--------------|
| 0.8  | v0.20+       |
| 0.7  | v0.11-0.19   |



Example code:
```rust

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
  some_test_field: String
}

fn my_source_stream() -> impl Stream<Item=MyTestStructure> {
  // Simulating a stream with a plain vector and throttling to show how it works
  stream::iter(vec![
    MyTestStructure {
      some_test_field: "test1".to_string()
    }; 1000
  ]).throttle(std::time::Duration::from_millis(50))
}

async fn test_json_array_stream() -> impl IntoResponse {
  StreamBodyAs::json_array(source_test_stream())
}

async fn test_json_nl_stream() -> impl IntoResponse {
  StreamBodyAs::json_nl(source_test_stream())
}

async fn test_csv_stream() -> impl IntoResponse {
  StreamBodyAs::csv(source_test_stream())
}

async fn test_text_stream() -> impl IntoResponse {
  StreamBodyAs::text(source_test_stream())
}

```

All examples available at [examples](examples) directory.

To run example use:
```
# cargo run --example json-example --features json
```

## Receiving a streamed request body

The mirror of the response side: an extractor that decodes an upload into a stream of items, so
a handler processes a body of any size a record at a time without holding it in memory.

```rust
use axum::extract::DefaultBodyLimit;
use axum_streams::{JsonNlStreamFrom, StreamBodyFromError};
use futures::StreamExt;

async fn ingest(mut items: JsonNlStreamFrom<MyTestStructure>) -> Result<Json<u64>, StreamBodyFromError> {
    let mut count = 0;
    while let Some(item) = items.next().await {
        let _item = item?;
        count += 1;
    }
    Ok(Json(count))
}

let app = Router::new()
    .route("/ingest", post(ingest))
    // A streaming upload is exactly what the default body limit exists to stop, so a route that
    // wants one has to say so. The extractor honours the limit when it is set.
    .layer(DefaultBodyLimit::disable());
```

There is one alias per format: `JsonNlStreamFrom`, `JsonArrayStreamFrom`, `CsvStreamFrom`,
`ProtobufStreamFrom`, `ArrowIpcStreamFrom`. Text has no inbound counterpart, because its framing
writes no delimiter and cannot be split back into items.

### Errors arrive in two places

A rejection has to be decided before any of the body is read, or the handler never gets to
stream it. So only what is knowable from the headers can be a rejection: an unrecognised or
missing `Content-Type` gives 415, and a declared `Content-Length` over the configured maximum
gives 413.

Everything else reaches the handler as an error item, because by then the handler already owns
the stream. `StreamBodyFromError` implements `IntoResponse`, so `item?` produces the right
status, provided the handler has not already begun writing its response.

Whether one bad record ends the stream depends on the format. JSON Lines and CSV frame records
independently, so a record that fails to deserialise is reported and reading continues. A JSON
array, protobuf stream or Arrow stream cannot resynchronise after a bad record, so the first one
ends the stream.

### Configuring the format

The extractor is built by axum, not by you, so there is nowhere to pass constructor arguments.
Attach the format to the route instead, which is the same idiom `DefaultBodyLimit` uses:

```rust
Router::new()
    .route("/ingest", post(ingest))
    .layer(Extension(StreamBodyFromConfig::new(
        CsvStreamFormat::new(true, b';'),
    )))
```

Without one, the format is built with its defaults. `StreamBodyFromOptions` carries the rest:
`max_obj_len` (one MiB by default, deliberately not unlimited for a body you did not produce),
`buf_capacity`, `max_body_len` and `validate_content_type`.

## Need client support?
There is the same functionality for:
- [reqwest-streams](https://github.com/abdolence/reqwest-streams-rs).

## Configuration of the frame size
By default, the library produces an HTTP frame per item in the stream. 
You can change this is using `StreamAsOptions`:

```rust
    StreamBodyAsOptions::new().buffering_ready_items(1000)
        .json_array(source_test_stream())
```

## Error handling
The library provides a way to propagate errors in the stream:

```rust
struct MyError {
    message: String,
}

impl Into<axum::Error> for MyError {
    fn into(self) -> axum::Error {
        axum::Error::new(self.message)
    }
}

fn my_source_stream() -> impl Stream<Item=Result<MyTestStructure, MyError>> {
  // Simulating a stream with a plain vector and throttling to show how it works
  stream::iter(vec![
    Ok(MyTestStructure {
      some_test_field: "test1".to_string()
    }); 1000
  ])
}

async fn test_json_array_stream() -> impl IntoResponse {
  // Use _with_errors functions or directly `StreamBodyAs::with_options` 
  // to produce a stream with errors
  StreamBodyAs::json_array_with_errors(source_test_stream())
}

```

### Observing errors

An error that happens mid-stream cannot be turned into an HTTP status code: the status and
headers have already been sent, so the response can only be terminated abnormally. Clients
(browsers, cURL, hyper) do detect this, but by default nothing tells you *why* it happened.

Use `on_error` to observe them. It is called for every error, both those coming from your
source stream and serialization errors produced by the format itself:

```rust
    StreamBodyAsOptions::new()
        .on_error(|err| tracing::error!("Stream failed: {err}"))
        .json_array(source_test_stream())
```

Alternatively, enable the `tracing` feature to have the library log them for you:

```toml
axum-streams = { version = "0.29", features = ["json", "tracing"] }
```

Errors are then logged at the `ERROR` level on the `http_streams_core` target, so they can be
filtered with `RUST_LOG=http_streams_core=off`. Both the log event and your `on_error`
callback fire when the feature is enabled and a callback is set.

Two things worth knowing:
- Bytes that were buffered but not yet flushed are discarded when an error occurs.
- If the client needs to distinguish a failed response from a complete one, model the failure
  in the item type itself (for example an untagged enum with an `error` variant), since a
  truncated response cannot carry that information reliably.

### Observing progress

A streaming response is polled after your handler has already returned, so nothing in the
handler can tell you how much of it actually went out. Enable the `tracing` feature to have the
library report that for you:

```toml
axum-streams = { version = "0.29", features = ["json", "tracing"] }
```

At `INFO` every response reports its totals once, when it ends:

```text
INFO http_streams_core::stream{format="json_array" direction="response" side="server" items=1000 bytes=28001 errors=0 elapsed_ms=11239 outcome="completed"}: Finished streaming an HTTP body items=1000 bytes=28001 errors=0 elapsed_ms=11239 outcome="completed"
```

The `outcome` tells apart the three ways a response can end: `completed`, `aborted` (the
client went away mid-stream, which is otherwise invisible), and `failed`, which reports at
`ERROR` instead, alongside the error itself.

Raise it to `RUST_LOG=axum_streams=debug,http_streams_core=debug` and long-running responses
additionally report progress about once a second:

```text
DEBUG http_streams_core::stream{format="json_array"}: Streaming an HTTP body items=91 bytes=2548 elapsed_ms=1008
DEBUG http_streams_core::stream{format="json_array"}: Streaming an HTTP body items=182 bytes=5096 elapsed_ms=2018
INFO  http_streams_core::stream{format="json_array" items=358 bytes=10024 errors=0 elapsed_ms=4000 outcome="aborted"}: Finished streaming an HTTP body items=358 bytes=10024 errors=0 elapsed_ms=4000 outcome="aborted"
```

Everything is recorded on an `http_streams_core::stream` span, created while your handler's
request span is still current, so collectors nest it under the request and read `items`,
`bytes`, `errors`, `elapsed_ms` and `outcome` as span attributes rather than as log text. Use
`http_streams_core=trace` to additionally get an event per frame.

The target is `http_streams_core` rather than `axum_streams` because the accounting is shared
with the client-side crate; the `direction` and `side` span fields tell the four cases apart.
Name both targets in your filter, so that anything this crate logs itself stays visible:

```text
RUST_LOG=axum_streams=debug,http_streams_core=debug
```

Reporting is time-based by default, so the number of lines is bound by how long a response runs
and not by how much it carries. Both triggers are configurable, and progress is also reported
whenever the item count crosses a step if you ask for one:

```rust
    StreamBodyAsOptions::new()
        .progress_interval(std::time::Duration::from_secs(5))
        .progress_items(100_000)
        .json_array(source_test_stream())
```

The same accounting is available without tracing, for metrics:

```rust
    StreamBodyAsOptions::new()
        .on_progress(|progress| {
            if progress.outcome != StreamBodyOutcome::InProgress {
                metrics::counter!("streamed_bytes").increment(progress.bytes);
            }
        })
        .json_array(source_test_stream())
```

`items` counts the objects successfully read from your source stream, so an item that failed is
not counted, and an item is whatever the format consumes: for the Arrow format that is a
`RecordBatch`, not a row. `bytes` counts what reached the HTTP layer, which is why it can be
lower than what was serialized when `buffering_bytes` discards a partial buffer on error.

Nothing is counted at all unless something is listening: with no `on_progress` callback and no
subscriber interested in `axum_streams` at all, the stream pipeline is left untouched.

## JSON array inside another object
Sometimes you need to include your array inside some object, e.g.:
```json
{
  "some_status_field": "ok",
  "data": [
    {
      "some_test_field": "test1"
    },
    {
      "some_test_field": "test2"
    }
  ]
}
```
The wrapping object that includes `data` field here is called envelope further.

You need to define both of your structures: envelope and records inside:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyEnvelopeStructure {
    something_else: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    data: Vec<MyItem>
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyItem {
  some_test_field: String
}
```

And use `json_array_with_envelope` instead of `json_array`.
Have a look at [json-array-complex-structure.rs](examples/json-array-complex-structure.rs) for detail example.

The support is limited:
- Only first level of nesting is supported to avoid complex implementation with performance impact. 
- You need either remove the target array field from `envelope` structure or use this Serde trick on the field to avoid JSON serialization issues:
```rust
    #[serde(skip_serializing_if = "Vec::is_empty")]
```

## Licence
Apache Software License (ASL)

## Author
Abdulla Abdurakhmanov
