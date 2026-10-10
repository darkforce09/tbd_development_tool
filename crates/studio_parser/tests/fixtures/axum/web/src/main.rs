use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let state = web::AppState::default();
    let app = web::prelude::router(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
