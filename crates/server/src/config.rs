use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub listen: SocketAddr,
    pub token: String,
}

fn default_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("cannot determine config directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

impl Config {
    pub fn load_or_init(path: Option<PathBuf>) -> Result<Config> {
        let path = match path {
            Some(p) => p,
            None => default_path()?,
        };
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            return Ok(toml::from_str(&raw)?);
        }
        let config = Config {
            listen: "127.0.0.1:7683".parse().unwrap(),
            token: uuid::Uuid::new_v4().simple().to_string(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(&config)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        eprintln!("octoterm: generated config at {} (token: {})", path.display(), config.token);
        Ok(config)
    }
}

/// 命令行 --host/--port 覆盖配置文件的 listen;None 表示沿用配置值
pub fn effective_listen(
    base: SocketAddr,
    host: Option<std::net::IpAddr>,
    port: Option<u16>,
) -> SocketAddr {
    let mut listen = base;
    if let Some(host) = host {
        listen.set_ip(host);
    }
    if let Some(port) = port {
        listen.set_port(port);
    }
    listen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SocketAddr {
        "127.0.0.1:7683".parse().unwrap()
    }

    #[test]
    fn no_overrides_keeps_config() {
        assert_eq!(effective_listen(base(), None, None), base());
    }

    #[test]
    fn host_and_port_override_independently() {
        assert_eq!(
            effective_listen(base(), Some("0.0.0.0".parse().unwrap()), None).to_string(),
            "0.0.0.0:7683"
        );
        assert_eq!(
            effective_listen(base(), None, Some(9000)).to_string(),
            "127.0.0.1:9000"
        );
    }

    #[test]
    fn both_overrides_apply() {
        assert_eq!(
            effective_listen(base(), Some("::1".parse().unwrap()), Some(9000)).to_string(),
            "[::1]:9000"
        );
    }
}
