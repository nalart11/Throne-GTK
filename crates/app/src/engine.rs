//! Фоновый исполнитель: всё, что нельзя делать в потоке интерфейса.
//!
//! Интерфейс не ждёт ядро и не ходит в сеть сам. Он шлёт [`Command`] и
//! получает [`Event`] — так подключение, тест двух сотен серверов и обновление
//! подписки не морозят окно.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use throne_config::generate::GeneratedConfig;
use throne_config::{subscription, Profile, Settings};
use throne_ipc::pb;
use throne_ipc::Core;
use tokio::sync::mpsc;

/// Что интерфейс просит сделать.
#[derive(Debug)]
pub enum Command {
    Connect {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    /// Подключение с автовыбором: ядро само держит лучший сервер группы.
    ConnectAuto {
        profiles: Vec<Profile>,
        /// Группа, к которой относится автовыбор — нужна интерфейсу, чтобы
        /// вернуть отметку на правильную строку.
        gid: i64,
        settings: Box<Settings>,
    },
    /// Перепроверить всех участников автовыбора прямо сейчас.
    AutoRecheck,
    /// Закрепить участника вручную; пустая строка снимает закрепление.
    AutoPin(String),
    Disconnect,
    /// Прогнать задержку по списку профилей. `ids` идёт параллельно `profiles`
    /// и возвращается в событиях, чтобы интерфейс знал, чью строку обновлять.
    TestLatency {
        profiles: Vec<Profile>,
        ids: Vec<i64>,
        settings: Box<Settings>,
    },
    StopTest,
    /// Замер скорости одного сервера. Идёт в своём ядре и не трогает рабочее
    /// соединение — как и проверка задержек.
    SpeedTest {
        profile: Box<Profile>,
        id: i64,
        settings: Box<Settings>,
    },
    FetchSubscription {
        gid: i64,
        url: String,
        user_agent: String,
    },
    /// Проверить, что конфиг вообще собирается и принимается ядром.
    CheckConfig {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    Shutdown,
}

/// Состояние соединения — то, что показывает главный экран.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected { profile_id: i64, name: String },
    Failed(String),
}

/// Что произошло в фоне.
#[derive(Debug)]
pub enum Event {
    Status(Status),
    /// Байты за последний интервал: скорость считается интерфейсом.
    Traffic { down: i64, up: i64 },
    Connections(Vec<Connection>),
    /// Состояние автовыбора: кто выбран сейчас и сколько серверов живо.
    AutoStatus {
        selected: String,
        pinned: String,
        alive: i32,
        total: i32,
        suspended: bool,
    },
    LatencyResult { id: i64, latency: i32 },
    TestFinished { tested: usize },
    /// Промежуточное состояние замера: ядро отдаёт его по запросу, пока идёт
    /// прогон, — иначе минуту непонятно, жив ли замер.
    SpeedProgress { id: i64, stage: String },
    SpeedResult {
        id: i64,
        download: String,
        upload: String,
        country: String,
        latency: i32,
        error: String,
    },
    Subscription {
        gid: i64,
        result: Result<subscription::Parsed, String>,
    },
    ConfigChecked(Result<(), String>),
    Log(String),
    Error(String),
}

/// Живое соединение в удобной для таблицы форме.
#[derive(Debug, Clone)]
pub struct Connection {
    pub id: String,
    pub dest: String,
    pub domain: String,
    pub network: String,
    pub process: String,
    pub upload: i64,
    pub download: i64,
    pub created_at: i64,
}

pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
}

impl Engine {
    /// Поднимает рабочий поток с собственным исполнителем tokio.
    /// `events` — сторона отправки; интерфейс держит приёмник.
    pub fn start(core_bin: PathBuf, runtime_dir: PathBuf, events: async_channel::Sender<Event>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("throne-engine".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("не удалось создать исполнитель tokio");
                runtime.block_on(run(core_bin, runtime_dir, rx, events));
            })
            .expect("не удалось запустить фоновый поток");
        Self { commands: tx }
    }

    pub fn send(&self, command: Command) {
        // Провал возможен только после Shutdown — тогда команда уже не нужна.
        let _ = self.commands.send(command);
    }
}

/// Ядро живёт ровно столько, сколько длится соединение: держать его
/// запущенным вхолостую незачем — оно тянет память под sing-box и Xray.
struct Running {
    core: Core,
    profile_id: i64,
    tags: Vec<String>,
    /// Теги участников автовыбора в порядке профилей: по ним ответ ядра
    /// превращается обратно в имя сервера.
    auto_members: Vec<(String, String)>,
}

async fn run(
    core_bin: PathBuf,
    runtime_dir: PathBuf,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: async_channel::Sender<Event>,
) {
    let mut running: Option<Running> = None;
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Shutdown => {
                        if let Some(r) = running.take() {
                            r.core.shutdown().await;
                        }
                        break;
                    }
                    Command::Disconnect => {
                        if let Some(r) = running.take() {
                            r.core.shutdown().await;
                        }
                        let _ = events.send(Event::Status(Status::Disconnected)).await;
                    }
                    Command::ConnectAuto { profiles, gid, settings } => {
                        if let Some(r) = running.take() {
                            r.core.shutdown().await;
                        }
                        let _ = events.send(Event::Status(Status::Connecting)).await;
                        match connect_auto(&core_bin, &runtime_dir, &profiles, gid, &settings).await {
                            Ok(new) => {
                                let _ = events
                                    .send(Event::Status(Status::Connected {
                                        profile_id: -gid,
                                        name: format!("Автовыбор из {}", profiles.len()),
                                    }))
                                    .await;
                                running = Some(new);
                            }
                            Err(e) => {
                                let _ = events
                                    .send(Event::Status(Status::Failed(format!("{e:#}"))))
                                    .await;
                            }
                        }
                    }
                    Command::AutoRecheck => {
                        if let Some(r) = &running {
                            let _ = r
                                .core
                                .client
                                .auto_selector_action(pb::AutoSelectorActionRequest {
                                    tag: Some(throne_config::generate::tags::PROXY.to_string()),
                                    action: Some("recheck".into()),
                                    member: None,
                                })
                                .await;
                        }
                    }
                    Command::AutoPin(member) => {
                        if let Some(r) = &running {
                            // Пустой member снимает закрепление — так это
                            // понимает и само ядро.
                            let tag = r
                                .auto_members
                                .iter()
                                .find(|(_, name)| *name == member)
                                .map(|(tag, _)| tag.clone())
                                .unwrap_or_default();
                            let _ = r
                                .core
                                .client
                                .auto_selector_action(pb::AutoSelectorActionRequest {
                                    tag: Some(throne_config::generate::tags::PROXY.to_string()),
                                    action: Some("select".into()),
                                    member: Some(tag),
                                })
                                .await;
                        }
                    }
                    Command::Connect { profile, settings } => {
                        if let Some(r) = running.take() {
                            r.core.shutdown().await;
                        }
                        let _ = events.send(Event::Status(Status::Connecting)).await;
                        match connect(&core_bin, &runtime_dir, &profile, &settings).await {
                            Ok(new) => {
                                let _ = events
                                    .send(Event::Status(Status::Connected {
                                        profile_id: profile.id,
                                        name: profile.name.clone(),
                                    }))
                                    .await;
                                running = Some(new);
                            }
                            Err(e) => {
                                let _ = events
                                    .send(Event::Status(Status::Failed(format!("{e:#}"))))
                                    .await;
                            }
                        }
                    }
                    Command::CheckConfig { profile, settings } => {
                        let outcome = check_config(&core_bin, &runtime_dir, &profile, &settings)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        let _ = events.send(Event::ConfigChecked(outcome)).await;
                    }
                    Command::TestLatency { profiles, ids, settings } => {
                        // Тест поднимает собственное ядро и не трогает рабочее:
                        // проверять серверы можно, не разрывая соединение.
                        let events = events.clone();
                        let core_bin = core_bin.clone();
                        let runtime_dir = runtime_dir.clone();
                        tokio::spawn(async move {
                            if let Err(e) = test_latency(&core_bin, &runtime_dir, profiles, ids, &settings, &events).await {
                                let _ = events.send(Event::Error(format!("{e:#}"))).await;
                                let _ = events.send(Event::TestFinished { tested: 0 }).await;
                            }
                        });
                    }
                    Command::SpeedTest { profile, id, settings } => {
                        let events = events.clone();
                        let core_bin = core_bin.clone();
                        let runtime_dir = runtime_dir.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                speed_test(&core_bin, &runtime_dir, *profile, id, &settings, &events).await
                            {
                                let _ = events
                                    .send(Event::SpeedResult {
                                        id,
                                        download: String::new(),
                                        upload: String::new(),
                                        country: String::new(),
                                        latency: 0,
                                        error: format!("{e:#}"),
                                    })
                                    .await;
                            }
                        });
                    }
                    Command::StopTest => {
                        if let Some(r) = &running {
                            let _ = r.core.client.stop_test().await;
                        }
                    }
                    Command::FetchSubscription { gid, url, user_agent } => {
                        let events = events.clone();
                        tokio::spawn(async move {
                            let result = fetch_subscription(&url, &user_agent)
                                .await
                                .map_err(|e| format!("{e:#}"));
                            let _ = events.send(Event::Subscription { gid, result }).await;
                        });
                    }
                }
            }

            _ = poll.tick() => {
                let Some(r) = &mut running else { continue };

                // Ядро могло упасть само — например, из-за отказа TUN.
                if let Some(status) = r.core.exited() {
                    let _ = events
                        .send(Event::Status(Status::Failed(format!(
                            "ядро завершилось ({status})"
                        ))))
                        .await;
                    running = None;
                    continue;
                }

                if let Ok(stats) = r.core.client.query_stats().await {
                    let sum = |m: &std::collections::HashMap<String, i64>| -> i64 {
                        r.tags.iter().filter_map(|t| m.get(t)).sum()
                    };
                    let (down, up) = (sum(&stats.downs), sum(&stats.ups));
                    if down != 0 || up != 0 {
                        let _ = events.send(Event::Traffic { down, up }).await;
                    }
                }

                if !r.auto_members.is_empty() {
                    if let Ok(response) = r.core.client.query_auto_selectors().await {
                        if let Some(group) = response.groups.first() {
                            let name_of = |tag: &str| -> String {
                                r.auto_members
                                    .iter()
                                    .find(|(member_tag, _)| member_tag == tag)
                                    .map(|(_, name)| name.clone())
                                    .unwrap_or_default()
                            };
                            let _ = events
                                .send(Event::AutoStatus {
                                    selected: name_of(group.selected()),
                                    pinned: name_of(group.pinned()),
                                    alive: group.members_alive(),
                                    total: group.members_total(),
                                    suspended: group.suspended(),
                                })
                                .await;
                        }
                    }
                }

                if let Ok(conns) = r.core.client.query_connections().await {
                    let list = conns
                        .active
                        .into_iter()
                        .map(|c| Connection {
                            id: c.id().to_string(),
                            dest: c.dest().to_string(),
                            domain: c.domain().to_string(),
                            network: c.network().to_string(),
                            process: c.process().to_string(),
                            upload: c.upload(),
                            download: c.download(),
                            created_at: c.created_at(),
                        })
                        .collect();
                    let _ = events.send(Event::Connections(list)).await;
                }

                let _ = r.profile_id;
            }
        }
    }
}

fn load_request(generated: &GeneratedConfig, settings: &Settings) -> pb::LoadConfigReq {
    let mut request = throne_ipc::load_request(generated.core_config.clone());
    request.disable_stats = Some(!settings.stats_enabled);
    request.need_xray = Some(generated.need_xray);
    request.xray_config = Some(generated.xray_config.clone());
    request.tun_ipv4_cidr = Some(generated.tun_ipv4_cidr.clone());
    request
}

async fn connect(
    core_bin: &PathBuf,
    runtime_dir: &PathBuf,
    profile: &Profile,
    settings: &Settings,
) -> Result<Running> {
    let generated = throne_config::generate(profile, settings).context("сборка конфига")?;
    let core = Core::spawn(core_bin, runtime_dir, settings.log_level == "debug").await?;

    if settings.mode.is_vpn() && !core.client.is_privileged().await.unwrap_or(false) {
        let core = core;
        core.shutdown().await;
        anyhow::bail!(
            "для режима VPN ядру нужны права на создание TUN-интерфейса.\n\
             Выдайте их один раз: sudo setcap cap_net_admin,cap_net_raw+ep {}",
            core_bin.display()
        );
    }

    core.client
        .start(load_request(&generated, settings))
        .await
        .context("запуск конфига в ядре")?;

    Ok(Running {
        core,
        profile_id: profile.id,
        tags: generated.tags,
        auto_members: Vec::new(),
    })
}

/// Подключение с автовыбором. Профили передаются уже отсортированными: их
/// порядок ядро принимает за исходное ранжирование.
async fn connect_auto(
    core_bin: &PathBuf,
    runtime_dir: &PathBuf,
    profiles: &[Profile],
    gid: i64,
    settings: &Settings,
) -> Result<Running> {
    if profiles.is_empty() {
        anyhow::bail!("в группе нет серверов");
    }
    let generated =
        throne_config::generate::generate_auto(profiles, settings).context("сборка конфига")?;
    let core = Core::spawn(core_bin, runtime_dir, settings.log_level == "debug").await?;

    if settings.mode.is_vpn() && !core.client.is_privileged().await.unwrap_or(false) {
        core.shutdown().await;
        anyhow::bail!(
            "для режима VPN ядру нужны права на создание TUN-интерфейса.\n\
             Выдайте их один раз: sudo setcap cap_net_admin,cap_net_raw+ep {}",
            core_bin.display()
        );
    }

    core.client
        .start(load_request(&generated, settings))
        .await
        .context("запуск конфига в ядре")?;

    let auto_members: Vec<(String, String)> = generated
        .tags
        .iter()
        .cloned()
        .zip(profiles.iter().map(|p| p.name.clone()))
        .collect();

    // Трафик ядро считает на том outbound-е, который реально дозвонился, а
    // не на группе: суммировать нужно и селектор, и всех его участников.
    let mut tags = vec![throne_config::generate::tags::PROXY.to_string()];
    tags.extend(auto_members.iter().map(|(tag, _)| tag.clone()));

    Ok(Running {
        core,
        profile_id: -gid,
        tags,
        auto_members,
    })
}

async fn check_config(
    core_bin: &PathBuf,
    runtime_dir: &PathBuf,
    profile: &Profile,
    settings: &Settings,
) -> Result<()> {
    let generated = throne_config::generate(profile, settings).context("сборка конфига")?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;
    let outcome = core.client.check_config(load_request(&generated, settings)).await;
    core.shutdown().await;
    outcome
}

async fn test_latency(
    core_bin: &PathBuf,
    runtime_dir: &PathBuf,
    profiles: Vec<Profile>,
    ids: Vec<i64>,
    settings: &Settings,
    events: &async_channel::Sender<Event>,
) -> Result<()> {
    if profiles.is_empty() {
        let _ = events.send(Event::TestFinished { tested: 0 }).await;
        return Ok(());
    }
    let generated = throne_config::generate_test(&profiles, settings).context("сборка конфига теста")?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;

    let request = pb::TestReq {
        config: Some(generated.core_config.clone()),
        outbound_tags: generated.tags.clone(),
        url: Some(settings.test_url.clone()),
        max_concurrency: Some(settings.test_concurrency),
        test_timeout_ms: Some(settings.test_timeout_ms),
        need_xray: Some(generated.need_xray),
        xray_config: Some(generated.xray_config.clone()),
        ..Default::default()
    };

    let outcome = core.client.test(request).await;
    let mut tested = 0;

    if let Ok(response) = &outcome {
        for result in &response.results {
            // Тег вида `p-<индекс>` привязывает результат к строке списка.
            let Some(index) = generated.tags.iter().position(|t| t == result.outbound_tag()) else {
                continue;
            };
            let Some(&id) = ids.get(index) else { continue };
            // Провал теста кодируем отрицательной задержкой: ноль означает
            // «ещё не проверяли», и путать эти два состояния нельзя.
            let latency = if result.error().is_empty() {
                result.latency_ms().max(1)
            } else {
                -1
            };
            tested += 1;
            let _ = events.send(Event::LatencyResult { id, latency }).await;
        }
    }

    core.shutdown().await;
    let _ = events.send(Event::TestFinished { tested }).await;
    outcome.map(|_| ())
}

/// Замер скорости. Ядро считает его само; наша задача — собрать конфиг,
/// пока идёт прогон опрашивать состояние и вернуть результат одной записью.
async fn speed_test(
    core_bin: &PathBuf,
    runtime_dir: &PathBuf,
    profile: Profile,
    id: i64,
    settings: &Settings,
    events: &async_channel::Sender<Event>,
) -> Result<()> {
    let profiles = [profile];
    let generated =
        throne_config::generate_test(&profiles, settings).context("сборка конфига замера")?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;

    let mut request = throne_ipc::speed_test_request(generated.core_config.clone());
    request.outbound_tags = generated.tags.clone();
    request.test_download = Some(settings.speed_test_download);
    request.test_upload = Some(settings.speed_test_upload);
    request.timeout_ms = Some(settings.speed_test_timeout_ms);
    request.need_xray = Some(generated.need_xray);
    request.xray_config = Some(generated.xray_config.clone());

    // Пока ядро занято замером, спрашиваем его о ходе дела.
    let progress = {
        let client = core.client.clone();
        let events = events.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(700));
            loop {
                tick.tick().await;
                let Ok(state) = client.query_speed_test().await else {
                    break;
                };
                if !state.is_running() {
                    continue;
                }
                let Some(result) = &state.result else { continue };
                let stage = if !result.dl_speed().is_empty() {
                    format!("приём {}", result.dl_speed())
                } else if !result.server_name().is_empty() {
                    format!("сервер {}", result.server_name())
                } else {
                    "замеряю…".to_string()
                };
                let _ = events.send(Event::SpeedProgress { id, stage }).await;
            }
        })
    };

    let outcome = core.client.speed_test(request).await;
    progress.abort();
    core.shutdown().await;

    let response = outcome?;
    let result = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("ядро не вернуло результат замера"))?;

    let _ = events
        .send(Event::SpeedResult {
            id,
            download: result.dl_speed().to_string(),
            upload: result.ul_speed().to_string(),
            country: result.server_country().to_string(),
            latency: result.latency(),
            error: result.error().to_string(),
        })
        .await;
    Ok(())
}

async fn fetch_subscription(url: &str, user_agent: &str) -> Result<subscription::Parsed> {
    let (body, userinfo) = subscription::fetch(url, user_agent, 30).await?;
    let mut parsed = subscription::parse(&body)?;
    if !userinfo.is_empty() {
        parsed.info = subscription::format_userinfo(&userinfo);
    }
    Ok(parsed)
}
