//! Loopback-only shared IPC/lifecycle smoke test; no profiles, Internet or TUN.
//! Run `just ipc-smoke` to preserve the core's exact parent-name check.

use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{ensure, Context, Result};
use nolurix_ipc::Core;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const LIMIT: Duration = Duration::from_secs(60);
const MARKER: &str = "nolurix-ipc-loopback-ok";

struct RuntimeDir(PathBuf);

impl RuntimeDir {
    fn new() -> Result<Self> {
        // Atomic create_dir, not create_dir_all: never reuse someone else's directory.
        for attempt in 0..32 {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nolurix-ipc-smoke-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        anyhow::bail!("could not allocate unique runtime directory")
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn proxy_roundtrip(port: u16) -> Result<()> {
    let origin = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = origin.local_addr()?;
    let serve = async {
        let (mut stream, _) = origin.accept().await?;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            ensure!(request.len() < 8192, "oversized origin request");
            request.push(stream.read_u8().await?);
        }
        ensure!(
            request.starts_with(b"GET /ipc-smoke HTTP/1."),
            "unexpected origin request"
        );
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{MARKER}",
                    MARKER.len()
                )
                .as_bytes(),
            )
            .await?;
        stream.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    };
    let fetch = async {
        // Raw HTTP proxy request: cannot inherit environment proxies or bypass loopback.
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(format!("GET http://{address}/ipc-smoke HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        let mut response = String::new();
        stream.take(16384).read_to_string(&mut response).await?;
        ensure!(
            response.starts_with("HTTP/1.1 200"),
            "proxy returned non-200 response"
        );
        ensure!(
            response.ends_with(MARKER),
            "loopback response marker missing"
        );
        Ok::<_, anyhow::Error>(())
    };
    tokio::try_join!(serve, fetch)?;
    Ok(())
}

async fn smoke(runtime: &RuntimeDir) -> Result<()> {
    let executable = std::env::current_exe()?;
    ensure!(
        executable.file_stem().is_some_and(|name| name == "nolurix"),
        "run via just ipc-smoke (exact parent name required)"
    );
    let binary = executable
        .parent()
        .context("missing executable parent")?
        .join(format!("nolurix-core{}", std::env::consts::EXE_SUFFIX));
    ensure!(
        binary.is_file(),
        "missing adjacent core: {}",
        binary.display()
    );

    let (first, second) = tokio::join!(
        Core::spawn(&binary, &runtime.0, false),
        Core::spawn(&binary, &runtime.0, false)
    );
    let first = first.context("first concurrent core")?;
    let second = second.context("second concurrent core")?;
    let (a, b) = tokio::join!(first.client.is_privileged(), second.client.is_privileged());
    a.context("first query")?;
    b.context("second query")?;

    // The parser has no bound-port query. Reserve/release an ephemeral loopback
    // port and retry start a bounded number of times if another process wins it.
    let mut selected = None;
    let mut last_error = None;
    for _ in 0..5 {
        let reservation = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        let config = serde_json::json!({
            "log": {"level": "error"},
            "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": port}],
            "outbounds": [{"type": "direct", "tag": "direct"}],
            "route": {"final": "direct"}
        });
        match first
            .client
            .start(nolurix_ipc::load_request(config.to_string()))
            .await
        {
            Ok(()) => {
                selected = Some(port);
                break;
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }
    let port = selected.with_context(|| {
        format!("starting loopback proxy after five port reservations: {last_error:?}")
    })?;
    proxy_roundtrip(port)
        .await
        .context("initial proxy request")?;

    let survivor = second.client.clone();
    timeout(Duration::from_secs(10), second.shutdown())
        .await
        .context("second shutdown timed out")?;
    ensure!(
        timeout(Duration::from_secs(2), survivor.is_privileged())
            .await
            .context("retained client did not become terminal")?
            .is_err(),
        "retained client still usable after shutdown"
    );
    proxy_roundtrip(port)
        .await
        .context("first core must survive second shutdown")?;
    drop(survivor);

    timeout(Duration::from_secs(10), first.shutdown())
        .await
        .context("first shutdown timed out")?;
    ensure!(
        TcpStream::connect(("127.0.0.1", port)).await.is_err(),
        "proxy listener survived shutdown"
    );
    ensure!(
        std::fs::read_dir(&runtime.0)?.next().is_none(),
        "IPC endpoints survived shutdown"
    );

    let third = Core::spawn(&binary, &runtime.0, false)
        .await
        .context("restart core")?;
    third
        .client
        .is_privileged()
        .await
        .context("restart query")?;
    timeout(Duration::from_secs(10), third.shutdown())
        .await
        .context("restart shutdown timed out")?;
    ensure!(
        std::fs::read_dir(&runtime.0)?.next().is_none(),
        "restart IPC endpoint survived shutdown"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let runtime = RuntimeDir::new()?;
    // Cancellation drops owned cores (kill-on-drop) and loopback sockets on every error.
    let outcome = timeout(LIMIT, smoke(&runtime))
        .await
        .context("IPC smoke exceeded 60 seconds")?;
    let path = runtime.0.clone();
    drop(runtime);
    ensure!(
        !path.exists(),
        "temporary runtime directory was not removed"
    );
    outcome?;
    println!("IPC smoke passed: concurrent cores, loopback HTTP proxy, isolated shutdown, restart, cleanup (no Internet/TUN)");
    Ok(())
}
