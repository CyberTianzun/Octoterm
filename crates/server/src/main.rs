use clap::Parser;
use octoterm_server::app::{serve, AppState};
use octoterm_server::config::{Config, WindowSize};
use octoterm_server::session::manager::SessionManager;

#[derive(clap::Subcommand)]
enum Cmd {
    /// agent 的 hook 回调客户端(由装进 agent 配置里的 hook 调用,不是给人敲的)。
    ///
    /// 只有不支持 http 型 hook 的 agent(如 Codex)需要它:那边只能执行一个命令,
    /// 于是让它执行我们自己,由我们转发到本地 server。
    Hook {
        /// `http://127.0.0.1:<port>/hook/<agent>/<event>`
        url: String,
    },
}

#[derive(Parser)]
#[command(name = "octoterm-server", about = "octoterm terminal session daemon")]
struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// 配置文件路径(缺省用平台配置目录)
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    /// 覆盖监听 IP(如 0.0.0.0;对外监听请自行保证网络层安全)
    #[arg(long)]
    host: Option<std::net::IpAddr>,
    /// 覆盖监听端口
    #[arg(long)]
    port: Option<u16>,
    /// 固定鉴权 token(缺省每次启动随机生成并打印)
    #[arg(long)]
    token: Option<String>,
    /// 多端同时 attach 一个会话时 pty 用谁的尺寸(缺省 smallest)
    #[arg(long, value_enum)]
    window_size: Option<WindowSize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // hook 子命令要在**一切之前**处理:它由 agent 在每个事件上同步调用,必须快、
    // 必须安静。初始化日志、读配置、起 runtime 对它都是纯负担,而任何一行多余的
    // stdout 输出都会被 agent 当成决策读走。
    if let Some(Cmd::Hook { url }) = Args::parse().cmd {
        std::process::exit(octoterm_server::agent::hook_cli::run(&url));
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let config = Config::load(args.config)?;
    let listen = octoterm_server::config::effective_listen(config.listen, args.host, args.port);
    let (token, generated) = octoterm_server::config::resolve_token(args.token, config.token);
    let manager = SessionManager::new(1 << 20, args.window_size.unwrap_or(config.window_size));
    let launchers = std::sync::Arc::new(octoterm_server::launcher::providers(&config.launchers));
    let listener = tokio::net::TcpListener::bind(listen).await?;

    // Jupyter 式:每次启动打印可直接点开的访问 URL(0.0.0.0/:: 显示为 127.0.0.1)
    let ip = listen.ip();
    let host_part = if ip.is_unspecified() {
        "127.0.0.1".to_string()
    } else if ip.is_ipv6() {
        format!("[{ip}]")
    } else {
        ip.to_string()
    };
    eprintln!("octoterm-server listening on {listen}");
    eprintln!("    http://{host_part}:{}/#token={token}", listen.port());
    if generated {
        eprintln!("    (token 为本次启动随机生成;用 --token 或配置文件可固定)");
    }
    let listen_port = listener.local_addr()?.port();
    let agents = config.agents;
    let agent_sessions = std::sync::Arc::new(octoterm_server::agent::store::AgentSessionStore::new());
    serve(listener, AppState { manager, token, launchers, listen_port, agents, agent_sessions }).await
}
