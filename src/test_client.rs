use axum::Router;
use reqwest::RequestBuilder;
use std::net::SocketAddr;
use tokio::net::TcpListener;

// This class was originally a copy from Axum project (https://github.com/tokio-rs/axum), since
// this not available for external crates to use in tests
pub(crate) struct TestClient {
    client: reqwest::Client,
    addr: SocketAddr,
}

impl TestClient {
    pub(crate) async fn new(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Could not bind ephemeral socket");
        let addr = listener.local_addr().unwrap().clone();
        println!("Listening on {}", addr);

        tokio::spawn(async move {
            let server = axum::serve(listener, router);
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
