//! The API server.

#[tokio::main]
async fn main() {
    let dev = std::env::var("DEV").is_ok();
    let demo = std::env::var("DEMO").is_ok();
    let app = api::router(dev, demo);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
