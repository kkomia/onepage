mod access;
mod auth;
mod config;
mod db;
mod error;
mod logging;
mod models;
mod routes;
mod services;
mod state;
mod templates;
mod util;
mod web;

#[cfg(windows)]
mod service;

use std::net::SocketAddr;
use std::process::ExitCode;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::config::{Cli, Options};
use crate::state::AppState;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("OnePage 启动失败: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match config::parse_cli()? {
        Cli::Help => {
            print!("{}", config::help_text());
            Ok(())
        }
        Cli::Version => {
            println!("OnePage {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Cli::Run(options) => run_with_options(options),
    }
}

fn run_with_options(options: Options) -> Result<()> {
    if options.service {
        #[cfg(windows)]
        {
            service::run()?;
            return Ok(());
        }
        #[cfg(not(windows))]
        anyhow::bail!("--service 仅支持 Windows");
    }

    let config = config::Config::load(&options)?;
    let _log_guard = logging::init(&config, false)?;
    init_panic_hook();

    let runtime = build_runtime()?;
    runtime.block_on(async {
        let (state, listener) = prepare(&config).await?;
        tracing::info!(
            bind = %config.bind,
            data_dir = %config.data_dir.display(),
            "OnePage 已启动"
        );
        serve_prepared(state, listener, shutdown_signal()).await
    })
}

pub fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("无法创建异步运行时")
}

pub async fn prepare(config: &config::Config) -> Result<(AppState, TcpListener)> {
    let state = AppState::new(config.clone())?;
    match services::favicon::prune_orphan_icons(&state.db).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "已清理不再被任何链接引用的图标"),
        Err(error) => tracing::warn!(error = %error, "图标清理跳过"),
    }
    let listener = TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("无法监听 {}", config.bind))?;
    Ok((state, listener))
}

pub async fn serve_prepared<F>(state: AppState, listener: TcpListener, shutdown: F) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let app = routes::router(state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("HTTP 服务异常退出")?;
    tracing::info!("OnePage 已停止");
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let terminate = signal(SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    tracing::info!("收到停止信号，正在关闭");
}

fn init_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "进程发生未捕获异常");
        previous(info);
    }));
}
