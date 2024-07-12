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
axum-streams = { version = "0.18", features=["json", "csv", "protobuf", "text"] }
```

## Compatibility matrix

| axum | axum-streams |
|------|--------------|
| 0.7  | v0.11+       |
| 0.6  | v0.9-v0.10   |
| 0.5  | 0.7          |



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
