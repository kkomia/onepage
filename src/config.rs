use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

const HELP: &str = "\
OnePage — 团队网页导航

用法:
  onepage [--config <path>] [--service]

选项:
  -c, --config <path>  指定 onepage.toml 配置文件
      --service       以 Windows 服务方式运行（由 sc.exe 调用）
  -h, --help           显示帮助
  -V, --version        显示版本
";

#[derive(Debug, Clone)]
pub struct Options {
    pub config_path: Option<PathBuf>,
    pub service: bool,
}

pub enum Cli {
    Run(Options),
    Help,
    Version,
}

pub fn parse_cli() -> Result<Cli> {
    let mut config_path = None;
    let mut service = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "--service" => service = true,
            "-c" | "--config" => {
                config_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--config 需要一个文件路径"))?,
                ));
            }
            other if other.starts_with("--config=") => {
                config_path = Some(PathBuf::from(&other["--config=".len()..]));
            }
            other => bail!("未知参数: {other}（使用 --help 查看用法）"),
        }
    }

    if config_path.is_none() {
        if let Ok(path) = env::var("ONEPAGE_CONFIG") {
            if !path.trim().is_empty() {
                config_path = Some(PathBuf::from(path));
            }
        }
    }

    Ok(Cli::Run(Options {
        config_path,
        service,
    }))
}

pub fn help_text() -> &'static str {
    HELP
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    bind: Option<String>,
    data_dir: Option<String>,
    base_url: Option<String>,
    secure_cookies: Option<bool>,
    session_days: Option<i64>,
    log: Option<String>,
    icon_fetch: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub base_url: Option<String>,
    pub secure_cookies: bool,
    pub session_days: i64,
    pub log_filter: String,
    pub icon_fetch: bool,
    pub config_path: Option<PathBuf>,
    pub dev: bool,
}

impl Config {
    pub fn load(options: &Options) -> Result<Self> {
        let config_path = resolve_config_path(options.config_path.clone());
        let config_dir = config_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or(env::current_dir().context("无法读取当前目录")?);

        let file = match &config_path {
            Some(path) => {
                let raw = fs::read_to_string(path)
                    .with_context(|| format!("无法读取配置文件 {}", path.display()))?;
                toml::from_str::<FileConfig>(&raw)
                    .with_context(|| format!("配置文件格式错误 {}", path.display()))?
            }
            None => FileConfig::default(),
        };

        let bind_raw = env_or(file.bind, "ONEPAGE_BIND")
            .unwrap_or_else(|| "0.0.0.0:8686".to_string());
        let bind: SocketAddr = bind_raw
            .parse()
            .with_context(|| format!("bind 地址无效: {bind_raw}"))?;

        let data_raw = env_or(file.data_dir, "ONEPAGE_DATA_DIR")
            .unwrap_or_else(|| "data".to_string());
        let mut data_dir = PathBuf::from(&data_raw);
        if data_dir.is_relative() {
            data_dir = config_dir.join(data_dir);
        }

        let base_url = env_or(file.base_url, "ONEPAGE_BASE_URL")
            .map(|v| v.trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());

        let secure_cookies = env_bool("ONEPAGE_SECURE_COOKIES")
            .or(file.secure_cookies)
            .unwrap_or_else(|| {
                base_url
                    .as_deref()
                    .map(|u| u.starts_with("https://"))
                    .unwrap_or(false)
            });

        let session_days = env::var("ONEPAGE_SESSION_DAYS")
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .or(file.session_days)
            .unwrap_or(30)
            .clamp(1, 365);

        let log_filter = env_or(file.log, "ONEPAGE_LOG").unwrap_or_else(|| "info".to_string());
        let icon_fetch = env_bool("ONEPAGE_ICON_FETCH")
            .or(file.icon_fetch)
            .unwrap_or(true);

        fs::create_dir_all(&data_dir)
            .with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
        let logs_dir = data_dir.join("logs");
        fs::create_dir_all(&logs_dir)
            .with_context(|| format!("无法创建日志目录 {}", logs_dir.display()))?;

        Ok(Self {
            bind,
            data_dir,
            logs_dir,
            base_url,
            secure_cookies,
            session_days,
            log_filter,
            icon_fetch,
            config_path,
            dev: cfg!(debug_assertions),
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("onepage.db")
    }
}

fn resolve_config_path(given: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = given {
        return Some(path);
    }

    let cwd = env::current_dir().ok()?.join("onepage.toml");
    if cwd.is_file() {
        return Some(cwd);
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside_exe = dir.join("onepage.toml");
            if beside_exe.is_file() {
                return Some(beside_exe);
            }
        }
    }

    None
}

fn env_or(value: Option<String>, key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or(value)
        .map(|v| v.trim().to_string())
}

fn env_bool(key: &str) -> Option<bool> {
    let value = env::var(key).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
