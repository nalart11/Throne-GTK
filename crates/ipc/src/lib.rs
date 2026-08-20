//! Транспорт до ядра ThroneGtkCore.
//!
//! Ядро не поднимает сокет само: оно подключается к нашему, проверяя, что peer
//! по SO_PEERCRED — его собственный родитель, и что бинарь родителя лежит рядом
//! и называется `throne-gtk`. Поэтому GUI слушает, а ядро дозванивается.
//!
//! Кадры (little-endian):
//!   запрос : [u32 req_id][u16 method_len][method][u32 payload_len][payload]
//!   ответ  : [u32 req_id][u8 status][u32 data_len][data]
//! status != 0 — тело ответа это текст ошибки.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/libcore.rs"));
}

/// Максимальный размер кадра. Ядро своё, но канал всё равно не должен уметь
/// заставить нас выделить произвольный объём памяти по битому заголовку.
const MAX_FRAME: u32 = 64 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<Result<Vec<u8>>>>>>;

struct Call {
    method: &'static str,
    payload: Vec<u8>,
    reply: oneshot::Sender<Result<Vec<u8>>>,
}

/// Клиент одного живого соединения с ядром. Клонируется свободно.
#[derive(Clone)]
pub struct CoreClient {
    tx: mpsc::Sender<Call>,
}

/// Запущенный процесс ядра вместе с каналом к нему.
pub struct Core {
    child: Child,
    socket_path: PathBuf,
    pub client: CoreClient,
}

impl Core {
    /// Поднимает сокет, запускает `core_bin` и ждёт, пока ядро дозвонится.
    pub async fn spawn(core_bin: &Path, socket_dir: &Path, debug: bool) -> Result<Self> {
        let socket_path = socket_dir.join(format!("core-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        std::fs::create_dir_all(socket_dir)
            .with_context(|| format!("создание {}", socket_dir.display()))?;

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("bind {}", socket_path.display()))?;

        let mut cmd = Command::new(core_bin);
        cmd.env("THRONE_CORE_SOCKET", &socket_path)
            .env("THRONE_CORE_DEBUG", if debug { "1" } else { "0" })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("запуск ядра {}", core_bin.display()))?;

        pipe_core_logs(&mut child);

        // Ядро ретраит подключение десять раз по 500 мс; ждём с запасом.
        let accept = tokio::time::timeout(std::time::Duration::from_secs(15), listener.accept());
        let stream = match accept.await {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return Err(e).context("accept от ядра");
            }
            Err(_) => {
                let _ = child.kill().await;
                bail!(
                    "ядро не подключилось к {} за 15 с — проверьте, что бинарь GUI называется \
                     `throne-gtk` и лежит рядом с ядром",
                    socket_path.display()
                );
            }
        };

        let client = CoreClient::attach(stream);
        Ok(Self {
            child,
            socket_path,
            client,
        })
    }

    pub async fn shutdown(mut self) {
        let _ = self.client.stop().await;
        let _ = self.child.kill().await;
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Успел ли процесс ядра умереть сам по себе.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn pipe_core_logs(child: &mut Child) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "core", "{line}");
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "core", "{line}");
            }
        });
    }
}

/// Запрос на запуск конфига.
///
/// Собирать `pb::LoadConfigReq` вручную опасно: ядро разыменовывает часть
/// полей без проверки на nil, и пропущенный `need_extra_process` роняет его
/// паникой уже после того, как конфиг прошёл проверку. Здесь все такие поля
/// заполнены всегда.
pub fn load_request(core_config: String) -> pb::LoadConfigReq {
    pb::LoadConfigReq {
        core_config: Some(core_config),
        need_extra_process: Some(false),
        need_xray: Some(false),
        xray_config: Some(String::new()),
        disable_stats: Some(false),
        tun_ipv4_cidr: Some(String::new()),
        ..Default::default()
    }
}

/// То же для запроса на проверку скорости: там ядро разыменовывает свой набор
/// полей.
pub fn speed_test_request(config: String) -> pb::SpeedTestRequest {
    pb::SpeedTestRequest {
        config: Some(config),
        test_download: Some(false),
        test_upload: Some(false),
        simple_download: Some(false),
        simple_download_addr: Some(String::new()),
        only_country: Some(false),
        country_concurrency: Some(4),
        timeout_ms: Some(10_000),
        need_xray: Some(false),
        xray_config: Some(String::new()),
        ..Default::default()
    }
}

impl CoreClient {
    fn attach(stream: UnixStream) -> Self {
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::channel::<Call>(64);
        let pending: Pending = Arc::default();
        let next_id = Arc::new(AtomicU32::new(1));

        // Писатель: кадрирует запросы и запоминает, кому вернуть ответ.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                while let Some(call) = rx.recv().await {
                    let id = next_id.fetch_add(1, Ordering::Relaxed);
                    pending.lock().await.insert(id, call.reply);

                    let m = call.method.as_bytes();
                    let mut frame =
                        Vec::with_capacity(4 + 2 + m.len() + 4 + call.payload.len());
                    frame.extend_from_slice(&id.to_le_bytes());
                    frame.extend_from_slice(&(m.len() as u16).to_le_bytes());
                    frame.extend_from_slice(m);
                    frame.extend_from_slice(&(call.payload.len() as u32).to_le_bytes());
                    frame.extend_from_slice(&call.payload);

                    if writer.write_all(&frame).await.is_err() {
                        if let Some(reply) = pending.lock().await.remove(&id) {
                            let _ = reply.send(Err(anyhow!("соединение с ядром разорвано")));
                        }
                        break;
                    }
                }
            });
        }

        // Читатель: разбирает ответы и будит ждущих по req_id.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                loop {
                    let mut head = [0u8; 9];
                    if reader.read_exact(&mut head).await.is_err() {
                        break;
                    }
                    let id = u32::from_le_bytes(head[0..4].try_into().unwrap());
                    let status = head[4];
                    let len = u32::from_le_bytes(head[5..9].try_into().unwrap());
                    if len > MAX_FRAME {
                        break;
                    }
                    let mut data = vec![0u8; len as usize];
                    if len > 0 && reader.read_exact(&mut data).await.is_err() {
                        break;
                    }
                    if let Some(reply) = pending.lock().await.remove(&id) {
                        let _ = reply.send(if status == 0 {
                            Ok(data)
                        } else {
                            Err(anyhow!(String::from_utf8_lossy(&data).into_owned()))
                        });
                    }
                }
                // Соединение закрылось — никто из ждущих ответа уже не дождётся.
                for (_, reply) in pending.lock().await.drain() {
                    let _ = reply.send(Err(anyhow!("ядро закрыло соединение")));
                }
            });
        }

        Self { tx }
    }

    async fn call<Req: Message, Resp: Message + Default>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<Resp> {
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(Call {
                method,
                payload: req.encode_to_vec(),
                reply,
            })
            .await
            .map_err(|_| anyhow!("ядро не запущено"))?;
        let data = wait.await.map_err(|_| anyhow!("ответ ядра потерян"))??;
        Resp::decode(data.as_slice()).with_context(|| format!("разбор ответа {method}"))
    }

    /// Часть методов отвечает `ErrorResp`, где ошибка — обычное поле, а не
    /// ненулевой статус кадра. Разворачиваем её в `Err`, чтобы вызывающий код
    /// не проверял два разных канала ошибок.
    async fn call_checked<Req: Message>(&self, method: &'static str, req: Req) -> Result<()> {
        let resp: pb::ErrorResp = self.call(method, req).await?;
        match resp.error() {
            "" => Ok(()),
            e => Err(anyhow!(e.to_string())),
        }
    }

    pub async fn start(&self, req: pb::LoadConfigReq) -> Result<()> {
        self.call_checked("Start", req).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.call_checked("Stop", pb::EmptyReq {}).await
    }

    pub async fn check_config(&self, req: pb::LoadConfigReq) -> Result<()> {
        self.call_checked("CheckConfig", req).await
    }

    pub async fn test(&self, req: pb::TestReq) -> Result<pb::TestResp> {
        self.call("Test", req).await
    }

    pub async fn stop_test(&self) -> Result<()> {
        let _: pb::EmptyResp = self.call("StopTest", pb::EmptyReq {}).await?;
        Ok(())
    }

    pub async fn query_url_test(&self) -> Result<pb::QueryUrlTestResponse> {
        self.call("QueryURLTest", pb::EmptyReq {}).await
    }

    pub async fn ip_test(&self, req: pb::IpTestRequest) -> Result<pb::IpTestResp> {
        self.call("IPTest", req).await
    }

    pub async fn query_ip_test(&self) -> Result<pb::QueryIpTestResponse> {
        self.call("QueryIPTest", pb::EmptyReq {}).await
    }

    pub async fn speed_test(&self, req: pb::SpeedTestRequest) -> Result<pb::SpeedTestResponse> {
        self.call("SpeedTest", req).await
    }

    pub async fn query_speed_test(&self) -> Result<pb::QuerySpeedTestResponse> {
        self.call("QuerySpeedTest", pb::EmptyReq {}).await
    }

    pub async fn query_country_test(&self) -> Result<pb::QueryCountryTestResponse> {
        self.call("QueryCountryTest", pb::EmptyReq {}).await
    }

    pub async fn query_stats(&self) -> Result<pb::QueryStatsResp> {
        self.call("QueryStats", pb::EmptyReq {}).await
    }

    pub async fn query_connections(&self) -> Result<pb::QueryConnectionsResp> {
        self.call("QueryConnections", pb::EmptyReq {}).await
    }

    pub async fn query_auto_selectors(&self) -> Result<pb::QueryAutoSelectorsResponse> {
        self.call("QueryAutoSelectors", pb::EmptyReq {}).await
    }

    pub async fn auto_selector_action(&self, req: pb::AutoSelectorActionRequest) -> Result<()> {
        self.call_checked("AutoSelectorAction", req).await
    }

    pub async fn is_privileged(&self) -> Result<bool> {
        let r: pb::IsPrivilegedResponse = self.call("IsPrivileged", pb::EmptyReq {}).await?;
        Ok(r.has_privilege())
    }

    pub async fn set_system_dns(&self, clear: bool) -> Result<()> {
        let _: pb::EmptyResp = self
            .call(
                "SetSystemDNS",
                pb::SetSystemDnsRequest { clear: Some(clear) },
            )
            .await?;
        Ok(())
    }

    pub async fn default_interface(&self) -> Result<(String, i32)> {
        let r: pb::GetDefaultInterfaceResponse =
            self.call("GetDefaultInterface", pb::EmptyReq {}).await?;
        Ok((r.name().to_string(), r.index()))
    }

    pub async fn gen_wg_keypair(&self) -> Result<(String, String)> {
        let r: pb::GenWgKeyPairResponse = self.call("GenWgKeyPair", pb::EmptyReq {}).await?;
        match r.error() {
            "" => Ok((r.private_key().to_string(), r.public_key().to_string())),
            e => Err(anyhow!(e.to_string())),
        }
    }
}
