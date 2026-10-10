pub mod debug;

pub async fn health() -> &'static str {
    "ok"
}
