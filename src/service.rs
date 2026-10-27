#![cfg(windows)]

use std::ffi::OsString;
use std::fs;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

use crate::config::{Cli, Config};

const SERVICE_NAME: &str = "OnePage";

windows_service::define_windows_service!(ffi_service_main, service_main);

pub fn run() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("无法注册 Windows 服务入口")?;
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        write_startup_error(&format!("{error:#}"));
    }
}

fn run_service() -> Result<()> {
    let options = match crate::config::parse_cli()? {
        Cli::Run(options) => options,
        _ => anyhow::bail!("服务模式不能使用帮助或版本参数"),
    };
    let config = Config::load(&options)?;
    let _log_guard = crate::logging::init(&config, true)?;
    crate::init_panic_hook();

    let (stop_sender, stop_receiver) = watch::channel(false);
    let event_handler = move |event| -> ServiceControlHandlerResult {
        match event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_sender.send(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .context("无法注册服务控制处理器")?;
    set_status(
        &status_handle,
        ServiceState::StartPending,
        Duration::from_secs(20),
    )?;

    let runtime = crate::build_runtime()?;
    runtime.block_on(async move {
        let (state, listener) = crate::prepare(&config).await?;
        set_status(
            &status_handle,
            ServiceState::Running,
            Duration::from_secs(0),
        )?;
        tracing::info!(bind = %config.bind, "OnePage Windows 服务已启动");

        let result = crate::serve_prepared(state, listener, wait_for_stop(stop_receiver)).await;
        if let Err(error) = &result {
            tracing::error!(error = %error, "Windows 服务异常退出");
        }
        let _ = set_status(
            &status_handle,
            ServiceState::Stopped,
            Duration::from_secs(0),
        );
        result
    })
}

fn set_status(
    handle: &service_control_handler::ServiceStatusHandle,
    state: ServiceState,
    wait_hint: Duration,
) -> windows_service::Result<()> {
    let controls = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };
    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: controls,
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    })
}

async fn wait_for_stop(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
    tracing::info!("收到 Windows 服务停止信号");
}

fn write_startup_error(message: &str) {
    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("onepage-startup-error.log")));
    if let Some(path) = path {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = fs::write(path, format!("[{timestamp}] {message}\n"));
    }
    eprintln!("OnePage 服务启动失败: {message}");
}
