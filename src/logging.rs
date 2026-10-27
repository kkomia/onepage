use anyhow::Result;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

pub struct LogGuard {
    _file: WorkerGuard,
}

pub fn init(config: &Config, service: bool) -> Result<LogGuard> {
    let file_appender = rolling::daily(&config.logs_dir, "onepage.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let mut filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));
    filter = filter
        .add_directive("hyper=warn".parse().expect("static directive"))
        .add_directive("tower_http=info".parse().expect("static directive"));

    if service {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(true)
            .with_writer(file_writer)
            .try_init()
            .map_err(|error| anyhow::anyhow!("初始化日志失败: {error}"))?;
    } else {
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(file_writer)
            .with_ansi(false)
            .with_target(true);
        let console_layer = tracing_subscriber::fmt::layer()
            .with_ansi(true)
            .with_target(true);
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(console_layer)
            .try_init()
            .map_err(|error| anyhow::anyhow!("初始化日志失败: {error}"))?;
    }

    Ok(LogGuard { _file: guard })
}
