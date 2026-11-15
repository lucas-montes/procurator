use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(config_path) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("Config path must be provided as the first argument");
        std::process::exit(1);
    };

    // Logging first, from the config's `log` section (JSON on stdout by
    // default, see `context/specs/log-events.md`).
    let _log_guard = worker::logging::init(
        &config_path,
        "debug,sqlx=warn,hyper=warn,h2=warn,tower=warn,capnp_rpc=warn",
    );

    worker::ch_main(config_path).await;
}
