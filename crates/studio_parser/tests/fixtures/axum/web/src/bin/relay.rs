//! A second bin whose router is built and turned into a make-service in one `let`.

async fn relay() {}

#[tokio::main]
async fn main() {
    let app = axum::Router::new()
        .route("/relay", axum::routing::get(relay))
        .fallback(relay)
        .into_make_service();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
}
