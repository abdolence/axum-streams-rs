use axum::body::HttpBody;
use http::Request;

use reqwest::RequestBuilder;
use std::net::{SocketAddr, TcpListener};
use tower::make::Shared;
use tower_service::Service;

// This class is a copy from Axum project (https://github.com/tokio-rs/axum), since
// this not available for external crates to use in tests
pub(crate) struct TestClient {
    client: reqwest::Client,
    addr: SocketAddr,
}

impl TestClient {
    pub(crate) fn new<S, ResBody>(svc: S) -> Self
    where
        S: Service<Request<hyper::body::Body>, Response = http::Response<ResBody>>
            + Clone
            + Send
            + 'static,
        ResBody: HttpBody + Send + 'static,
        ResBody::Data: Send,
        ResBody::Error: Into<axum::BoxError>,
        S::Future: Send,
        S::Error: Into<axum::BoxError>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Could not bind ephemeral socket");
        let addr = listener.local_addr().unwrap();
        println!("Listening on {}", addr);

        tokio::spawn(async move {
            let server = hyper::server::Server::from_tcp(listener)
                .unwrap()
                .serve(Shared::new(svc));
            server.await.expect("server error");
        });

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        TestClient { client, addr }
    }

    pub(crate) fn get(&self, url: &str) -> RequestBuilder {
        self.client.get(format!("http://{}{}", self.addr, url))
    }
}
