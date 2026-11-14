use std::ffi::{c_char, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const HELPER_SOCKET: &str = "/Library/Application Support/Nolurix/run/helper.sock";
const BUNDLE: &str = "/Library/Application Support/Nolurix/Nolurix.app";
const CORE: &str = "/Library/Application Support/Nolurix/Nolurix.app/Contents/MacOS/nolurix-core";
const STATUS_NOT_REGISTERED: i64 = 0;
const STATUS_ENABLED: i64 = 1;
const STATUS_REQUIRES_APPROVAL: i64 = 2;

unsafe extern "C" {
    fn nolurix_helper_status() -> i64;
    fn nolurix_helper_register(error: *mut c_char, error_size: usize) -> bool;
    fn nolurix_helper_unregister(error: *mut c_char, error_size: usize) -> bool;
    fn nolurix_helper_open_settings();
    fn nolurix_helper_safe_acl(path: *const c_char) -> bool;
    fn nolurix_helper_signature(pid: libc::c_int) -> bool;
}

/// Unregister Service Management in the calling user's context. Native code
/// bounds the wait to ten seconds; call before starting GTK, without sudo.
pub fn unregister_helper() -> Result<()> {
    if unsafe { libc::getuid() == 0 || libc::geteuid() == 0 } {
        bail!("Run --unregister-helper as the logged-in user, without sudo");
    }
    let mut error = [0 as c_char; 1024];
    if !unsafe { nolurix_helper_unregister(error.as_mut_ptr(), error.len()) } {
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
        bail!("Service Management unregister: {message}");
    }
    Ok(())
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidpath(pid: libc::c_int, buffer: *mut c_void, size: u32) -> libc::c_int;
}

pub async fn start_privileged_core(endpoint: &Path, debug: bool) -> Result<u32> {
    tokio::time::timeout(Duration::from_secs(15), start_inner(endpoint, debug))
        .await
        .context("root-helper control timeout")?
}

async fn start_inner(endpoint: &Path, debug: bool) -> Result<u32> {
    // The unsigned local installer registers the daemon directly with launchd.
    // Prefer that already-running helper so SMAppService never has to evaluate
    // an ad-hoc signature. Protected PKG installations keep the normal
    // Service Management registration path.
    // Check installation before registering or sending anything to a daemon.
    validate_installation()?;
    validate_helper_parent()?;
    // Registration is allowed only when no endpoint exists, never after peer
    // authentication or trust-policy failure.
    let mut stream = match tokio::net::UnixStream::connect(HELPER_SOCKET).await {
        Ok(stream) => {
            validate_helper_endpoint()?;
            authenticate_helper(&stream)?;
            stream
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            if manual_installation()? {
                bail!(
                    "Manual root-helper is unavailable; repair/restart the manual launchd installation or reinstall the PKG. Automatic Service Management registration is disabled for this installation"
                );
            }
            // Service Management is synchronous FFI; keep it off the runtime.
            // Timeout bounds this caller, but cannot cancel an OS registration
            // call already running on the blocking pool.
            tokio::task::spawn_blocking(ensure_registered).await??;
            connect_helper().await?
        }
        Err(error) => return Err(error).context("root-helper connection"),
    };
    let path = endpoint
        .to_str()
        .context("путь IPC-сокета содержит недопустимые символы")?
        .as_bytes();
    if path.len() > u16::MAX as usize {
        bail!("слишком длинный путь IPC-сокета");
    }

    let mut request = Vec::with_capacity(15 + path.len());
    // Versioned wire magic is intentionally stable across the product rename.
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
    let response: serde_json::Value =
        serde_json::from_slice(&message).context("invalid root-helper response envelope")?;
    if response.get("version").and_then(|v| v.as_u64()) != Some(1) {
        bail!("unsupported root-helper protocol version");
    }
    if status != 0 || response.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        bail!(
            "root-helper: {}",
            response
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("request rejected")
        );
    }
    let pid = response
        .get("pid")
        .and_then(|v| v.as_u64())
        .filter(|pid| *pid > 0 && *pid <= i32::MAX as u64)
        .context("root-helper returned invalid worker PID")?;
    Ok(pid as u32)
}

fn manual_installation() -> Result<bool> {
    let marker = Path::new(BUNDLE).parent().unwrap().join(".nolurix-manual");
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            protected_node(&marker, false)?;
            if !metadata.is_file() {
                bail!("manual backend marker is not a regular file");
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("read protected helper backend marker"),
    }
}

fn reject_acl(path: &Path) -> Result<()> {
    let name = CString::new(path.as_os_str().as_bytes())?;
    if !unsafe { nolurix_helper_safe_acl(name.as_ptr()) } {
        bail!("unsafe or unreadable ACL on {}", path.display());
    }
    Ok(())
}

fn protected_node(path: &Path, socket: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.uid() != 0 || metadata.file_type().is_symlink() {
        bail!("untrusted ownership or symlink: {}", path.display());
    }
    if socket {
        // The root-owned public control socket deliberately permits clients to
        // connect; its ancestors, not its write bits, prevent replacement.
        if !metadata.file_type().is_socket() {
            bail!("helper endpoint is not a socket");
        }
    } else if metadata.mode() & 0o022 != 0 || (!metadata.is_dir() && !metadata.is_file()) {
        bail!("unprotected installation path: {}", path.display());
    }
    reject_acl(path)
}

fn protected_ancestors(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        protected_node(ancestor, false)?;
    }
    Ok(())
}

fn validate_installation() -> Result<()> {
    protected_ancestors(Path::new(BUNDLE))
        .context("Install or repair the protected Nolurix bundle using the PKG")?;
    let mut pending = vec![PathBuf::from(BUNDLE)];
    while let Some(path) = pending.pop() {
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            let frameworks = Path::new(BUNDLE).join("Contents/Frameworks");
            let resolved = std::fs::canonicalize(&path)?;
            let target = std::fs::read_link(&path)?;
            let target = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap().join(target)
            };
            // Reject lexical escapes too, even if an outside link currently
            // points back inside the protected closure.
            let mut normalized = PathBuf::new();
            for component in target.components() {
                match component {
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    std::path::Component::CurDir => {}
                    part => normalized.push(part.as_os_str()),
                }
            }
            if !path.starts_with(&frameworks)
                || !resolved.starts_with(&frameworks)
                || !normalized.starts_with(&frameworks)
                || std::fs::symlink_metadata(&path)?.uid() != 0
            {
                bail!("untrusted framework symlink: {}", path.display());
            }
            continue;
        }
        protected_node(&path, false)?;
        if std::fs::symlink_metadata(&path)?.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
    }
    if !std::fs::metadata(CORE)?.is_file() {
        bail!("missing installed core");
    }
    Ok(())
}

fn validate_helper_parent() -> Result<()> {
    let parent = Path::new(HELPER_SOCKET).parent().unwrap();
    protected_ancestors(parent)
        .context("Install or repair the protected helper run directory using the PKG")?;
    if !std::fs::symlink_metadata(parent)?.is_dir() {
        bail!("Repair the helper run directory using the PKG: not a directory");
    }
    Ok(())
}

fn validate_helper_endpoint() -> Result<PathBuf> {
    validate_helper_parent()?;
    let endpoint = PathBuf::from(HELPER_SOCKET);
    protected_node(&endpoint, true)?;
    Ok(endpoint)
}

async fn checked_connect() -> Result<UnixStream> {
    let endpoint = validate_helper_endpoint()?;
    let stream = UnixStream::connect(endpoint).await?;
    authenticate_helper(&stream)?;
    Ok(stream)
}

fn authenticate_helper(stream: &UnixStream) -> Result<()> {
    if stream.peer_cred()?.uid() != 0 {
        bail!("root-helper peer is not root");
    }
    let pid = super::peer_pid(stream)?.context("missing root-helper peer PID")?;
    let mut buffer = [0u8; 4096];
    let len = unsafe { proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if len <= 0 {
        return Err(std::io::Error::last_os_error()).context("root-helper executable");
    }
    let end = buffer
        .iter()
        .position(|b| *b == 0)
        .context("unterminated helper executable path")?;
    if std::fs::canonicalize(Path::new(std::ffi::OsStr::from_bytes(&buffer[..end])))?
        != Path::new(CORE)
    {
        bail!("unexpected root-helper executable");
    }
    validate_installation()?;
    if !unsafe { nolurix_helper_signature(pid as libc::c_int) } {
        bail!("root-helper live signature validation failed");
    }
    Ok(())
}

fn ensure_registered() -> Result<()> {
    let mut status = unsafe { nolurix_helper_status() };
    if status == STATUS_NOT_REGISTERED {
        let mut error = vec![0_i8; 1024];
        if !unsafe { nolurix_helper_register(error.as_mut_ptr(), error.len()) } {
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
            bail!("не удалось зарегистрировать root-helper: {message}");
        }
        status = unsafe { nolurix_helper_status() };
    }
    if status == STATUS_REQUIRES_APPROVAL {
        unsafe { nolurix_helper_open_settings() };
        bail!(
            "разрешите Nolurix в «Системные настройки → Основные → Объекты входа и расширения», затем повторите подключение"
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
        match checked_connect().await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(last_error.unwrap()).context("root-helper не запустился")
}
