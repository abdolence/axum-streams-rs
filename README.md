[![Cargo](https://img.shields.io/crates/v/axum-streams.svg)](https://crates.io/crates/axum-streams)
![tests and formatting](https://github.com/abdolence/axum-streams-rs/workflows/tests%20&amp;%20formatting/badge.svg)
![security audit](https://github.com/abdolence/axum-streams-rs/workflows/security%20audit/badge.svg)

# axum streams for Rust

Library provides HTTP response streaming support for [axum web framework](https://github.com/tokio-rs/axum):
- JSON array stream format
- JSON lines stream format
- CSV stream
- Protobuf len-prefixed stream format

This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
and want to avoid huge memory allocation.

## Quick start

Cargo.toml:
```toml
[dependencies]
axum-streams = { version = "0.6", features=["json", "csv", "protobuf"] }
```

Example code:
```rust

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
  some_test_field: String
}

fn source_test_stream() -> BoxStream<'static, MyTestStructure> {
  // Simulating a stream with a plain vector and throttling to show how it works
  Box::pin(stream::iter(vec![
    MyTestStructure {
      some_test_field: "test1".to_string()
    }; 1000
  ]).throttle(std::time::Duration::from_millis(50)))
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

```

All examples available at [examples](examples) directory.

To run example use:
```
# cargo run --example json-example
```

## Need client support?
There is the same functionality for:
- [reqwest-streams](https://github.com/abdolence/reqwest-streams-rs).

## Licence
Apache Software License (ASL)

## Author
Abdulla Abdurakhmanov
