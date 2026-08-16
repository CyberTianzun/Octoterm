use clap::Parser;
use octoterm_server::app::{serve, AppState};
use octoterm_server::config::Config;
use octoterm_server::session::manager::SessionManager;

#[derive(Parser)]
#[command(name = "octoterm-server", about = "octoterm terminal session daemon")]
struct Args {
    /// 配置文件路径(缺省用平台配置目录)
    #[arg(long)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let config = Config::load_or_init(args.config)?;
    let manager = SessionManager::new(1 << 20);
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    eprintln!("octoterm-server listening on {}", config.listen);
    serve(listener, AppState { manager, token: config.token }).await
}
