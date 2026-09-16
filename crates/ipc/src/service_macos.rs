use std::ffi::{c_char, CStr};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const HELPER_SOCKET: &str = "/var/run/dev.nalart.ThroneGtk.helper.sock";
const STATUS_NOT_REGISTERED: i64 = 0;
const STATUS_ENABLED: i64 = 1;
const STATUS_REQUIRES_APPROVAL: i64 = 2;

unsafe extern "C" {
    fn throne_helper_status() -> i64;
    fn throne_helper_register(error: *mut c_char, error_size: usize) -> bool;
    fn throne_helper_open_settings();
}

pub async fn start_privileged_core(endpoint: &Path, debug: bool) -> Result<()> {
    // The unsigned local installer registers the daemon directly with launchd.
    // Prefer that already-running helper so SMAppService never has to evaluate
    // an ad-hoc signature. Drag-and-drop installations keep the normal
    // Service Management registration path.
    let mut stream = match UnixStream::connect(HELPER_SOCKET).await {
        Ok(stream) => stream,
        Err(_) => {
            ensure_registered()?;
            connect_helper().await?
        }
    };
    let path = endpoint
        .to_str()
        .context("путь IPC-сокета содержит недопустимые символы")?
        .as_bytes();
    if path.len() > u16::MAX as usize {
        bail!("слишком длинный путь IPC-сокета");
    }

    let mut request = Vec::with_capacity(15 + path.len());
    request.extend_from_slice(b"THRONE1\0");
    request.extend_from_slice(&std::process::id().to_le_bytes());
    request.push(u8::from(debug));
    request.extend_from_slice(&(path.len() as u16).to_le_bytes());
    request.extend_from_slice(path);
    stream
        .write_all(&request)
        .await
        .context("запрос root-helper")?;

    let status = stream.read_u8().await.context("ответ root-helper")?;
    let message_len = stream.read_u16_le().await.context("ответ root-helper")? as usize;
    let mut message = vec![0; message_len];
    stream
        .read_exact(&mut message)
        .await
        .context("ответ root-helper")?;
    if status != 0 {
        bail!("root-helper: {}", String::from_utf8_lossy(&message));
    }
    Ok(())
}

fn ensure_registered() -> Result<()> {
    let mut status = unsafe { throne_helper_status() };
    if status == STATUS_NOT_REGISTERED {
        let mut error = vec![0_i8; 1024];
        if !unsafe { throne_helper_register(error.as_mut_ptr(), error.len()) } {
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
            bail!("не удалось зарегистрировать root-helper: {message}");
        }
        status = unsafe { throne_helper_status() };
    }
    if status == STATUS_REQUIRES_APPROVAL {
        unsafe { throne_helper_open_settings() };
        bail!(
            "разрешите Throne GTK в «Системные настройки → Основные → Объекты входа и расширения», затем повторите подключение"
        );
    }
    if status != STATUS_ENABLED {
        bail!("root-helper недоступен (статус Service Management: {status})");
    }
    Ok(())
}

async fn connect_helper() -> Result<UnixStream> {
    let mut last_error = None;
    for _ in 0..20 {
        match UnixStream::connect(HELPER_SOCKET).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(last_error.unwrap()).context("root-helper не запустился")
}
