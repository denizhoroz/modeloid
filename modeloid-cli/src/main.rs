use modeloid_cli::cli::Cli;

// Load .env before the tokio runtime starts: setting env vars while other threads run is unsound.
fn main() {
    dotenvy::dotenv().ok();

    tokio::runtime::Runtime::new()
        .expect("failed to start tokio runtime")
        .block_on(Cli::new().collect());
}
