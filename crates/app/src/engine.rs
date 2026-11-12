//! Background executor: everything that cannot be done on the UI thread.
//!
//! The interface does not wait for the core or access the network itself. It sends
//! [`Command`] and receives [`Event`], so connecting, testing two hundred servers,
//! and updating a subscription do not freeze the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use throne_config::generate::GeneratedConfig;
use throne_config::{subscription, Profile, Settings};
use throne_ipc::pb;
use throne_ipc::Core;
use tokio::sync::mpsc;

/// What the interface requests.
#[derive(Debug)]
pub enum Command {
    Connect {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    /// Connect with auto-selection: the core keeps the best server in the group.
    ConnectAuto {
        profiles: Vec<Profile>,
        /// Group associated with auto-selection, needed so the interface can
        /// return the marker to the correct row.
        gid: i64,
        settings: Box<Settings>,
    },
    /// Recheck all auto-selection members right now.
    AutoRecheck,
    /// Pin a member manually; an empty string removes the pin.
    AutoPin(String),
    Disconnect,
    /// Test latency across the profile list. `ids` runs alongside `profiles` and
    /// is returned in events so the interface knows which row to update.
    TestLatency {
        profiles: Vec<Profile>,
        ids: Vec<i64>,
        settings: Box<Settings>,
    },
    StopTest,
    /// Measure one server’s speed. It runs in its own core and does not affect
    /// the active connection, just like latency testing.
    SpeedTest {
        profile: Box<Profile>,
        id: i64,
        settings: Box<Settings>,
    },
    FetchSubscription {
        gid: i64,
        url: String,
        user_agent: String,
        send_hwid: bool,
        custom_hwid_params: String,
    },
    /// Check that the configuration builds and is accepted by the core.
    CheckConfig {
        profile: Box<Profile>,
        settings: Box<Settings>,
    },
    Shutdown,
}

/// Connection state shown by the main screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected { profile_id: i64, name: String },
    Failed(String),
}

/// What happened in the background.
#[derive(Debug)]
pub enum Event {
    Status(Status),
    /// Bytes during the last interval; the interface calculates the speed.
    Traffic {
        down: i64,
        up: i64,
    },
    Connections(Vec<Connection>),
    /// Auto-selection state: who is selected and how many servers are alive.
    AutoStatus {
        selected: String,
        pinned: String,
        alive: i32,
        total: i32,
        suspended: bool,
    },
    LatencyResult {
        id: i64,
        latency: i32,
    },
    TestFinished {
        tested: usize,
    },
    /// Intermediate measurement state: the core returns it on request while the
    /// test runs, otherwise it is unclear for a minute whether the test is alive.
    SpeedProgress {
        id: i64,
        stage: String,
    },
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

/// A live connection in a table-friendly form.
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
    /// Starts the worker thread with its own Tokio executor.
    /// `events` is the sending side; the interface holds the receiver.
    pub fn start(
        core_bin: PathBuf,
        runtime_dir: PathBuf,
        events: async_channel::Sender<Event>,
    ) -> Self {
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
        // Failure is possible only after Shutdown, when the command is no longer needed.
        let _ = self.commands.send(command);
    }
}

/// The core lives exactly as long as the connection: there is no reason to keep
/// it running idle, as it consumes memory for sing-box and Xray.
struct Running {
    core: Core,
    profile_id: i64,
    tags: Vec<String>,
    /// Auto-selection member tags in profile order: they turn the core response
    /// back into a server name.
    auto_members: Vec<(String, String)>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum TestKey {
    Latency,
    Speed(i64),
}

struct TestWorker {
    generation: u64,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct TestEvents {
    generation: u64,
    sender: mpsc::UnboundedSender<(u64, Event)>,
}

impl TestEvents {
    async fn send(&self, event: Event) -> Result<(), mpsc::error::SendError<(u64, Event)>> {
        self.sender.send((self.generation, event))
    }
}

async fn cancel_tests(
    workers: &mut HashMap<TestKey, TestWorker>,
    events: &async_channel::Sender<Event>,
) {
    let drained: Vec<_> = workers.drain().collect();
    for (_, worker) in &drained {
        worker.task.abort();
    }
    for (key, worker) in drained {
        let _ = worker.task.await;
        let event = match key {
            TestKey::Latency => Event::TestFinished { tested: 0 },
            TestKey::Speed(id) => Event::SpeedResult {
                id,
                download: String::new(),
                upload: String::new(),
                country: String::new(),
                latency: 0,
                error: "отменено".into(),
            },
        };
        let _ = events.send(event).await;
    }
}

// Bound control-plane calls independently of the core's longer test timeout.
async fn control<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .context("ядро не отвечает")?
}

async fn run(
    core_bin: PathBuf,
    runtime_dir: PathBuf,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: async_channel::Sender<Event>,
) {
    let mut running: Option<Running> = None;
    let mut workers = HashMap::<TestKey, TestWorker>::new();
    let mut subscriptions = tokio::task::JoinSet::new();
    let mut subscription_generations = HashMap::new();
    let (test_tx, mut test_rx) = mpsc::unbounded_channel();
    let mut generation = 0u64;
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(outcome) = subscriptions.join_next(), if !subscriptions.is_empty() => {
                if let Ok((gid, generation, result)) = outcome {
                    if subscription_generations.get(&gid) == Some(&generation) {
                        subscription_generations.remove(&gid);
                        let _ = events.send(Event::Subscription { gid, result }).await;
                    }
                }
            }
            Some((generation, event)) = test_rx.recv() => {
                let key = workers.iter().find_map(|(key, worker)|
                    (worker.generation == generation).then_some(*key));
                if let Some(key) = key {
                    let finished = matches!(event, Event::TestFinished { .. } | Event::SpeedResult { .. });
                    if finished {
                        if let Some(worker) = workers.remove(&key) {
                            let _ = worker.task.await;
                        }
                    }
                    let _ = events.send(event).await;
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Shutdown => {
                        cancel_tests(&mut workers, &events).await;
                        if let Some(r) = running.take() {
                            r.core.shutdown().await;
                        }
                        break;
                    }
                    Command::Disconnect => {
                        cancel_tests(&mut workers, &events).await;
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
                            let _ = control(r
                                .core
                                .client
                                .auto_selector_action(pb::AutoSelectorActionRequest {
                                    tag: Some(throne_config::generate::tags::PROXY.to_string()),
                                    action: Some("recheck".into()),
                                    member: None,
                                }))
                                .await;
                        }
                    }
                    Command::AutoPin(member) => {
                        if let Some(r) = &running {
                            // An empty member removes the pin; the core understands
                            // it this way too.
                            let tag = r
                                .auto_members
                                .iter()
                                .find(|(_, name)| *name == member)
                                .map(|(tag, _)| tag.clone())
                                .unwrap_or_default();
                            let _ = control(r
                                .core
                                .client
                                .auto_selector_action(pb::AutoSelectorActionRequest {
                                    tag: Some(throne_config::generate::tags::PROXY.to_string()),
                                    action: Some("select".into()),
                                    member: Some(tag),
                                }))
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
                        // The test starts its own core and does not affect the active
                        // one, so servers can be checked without dropping the connection.
                        if let Some(worker) = workers.remove(&TestKey::Latency) {
                            let mut old = HashMap::from([(TestKey::Latency, worker)]);
                            cancel_tests(&mut old, &events).await;
                        }
                        generation += 1;
                        let events = TestEvents { generation, sender: test_tx.clone() };
                        let core_bin = core_bin.clone();
                        let runtime_dir = runtime_dir.clone();
                        let task = tokio::spawn(async move {
                            if let Err(e) = test_latency(&core_bin, &runtime_dir, profiles, ids, &settings, &events).await {
                                let _ = events.send(Event::Error(format!("{e:#}"))).await;
                                let _ = events.send(Event::TestFinished { tested: 0 }).await;
                            }
                        });
                        workers.insert(TestKey::Latency, TestWorker { generation, task });
                    }
                    Command::SpeedTest { profile, id, settings } => {
                        if let Some(worker) = workers.remove(&TestKey::Speed(id)) {
                            let mut old = HashMap::from([(TestKey::Speed(id), worker)]);
                            cancel_tests(&mut old, &events).await;
                        }
                        generation += 1;
                        let events = TestEvents { generation, sender: test_tx.clone() };
                        let core_bin = core_bin.clone();
                        let runtime_dir = runtime_dir.clone();
                        let task = tokio::spawn(async move {
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
                        workers.insert(TestKey::Speed(id), TestWorker { generation, task });
                    }
                    Command::StopTest => {
                        cancel_tests(&mut workers, &events).await;
                    }
                    Command::FetchSubscription {
                        gid,
                        url,
                        user_agent,
                        send_hwid,
                        custom_hwid_params,
                    } => {
                        generation += 1;
                        let request_generation = generation;
                        subscription_generations.insert(gid, request_generation);
                        subscriptions.spawn(async move {
                            let result = fetch_subscription(
                                &url,
                                &user_agent,
                                send_hwid,
                                &custom_hwid_params,
                            )
                                .await
                                .map_err(|e| format!("{e:#}"));
                            (gid, request_generation, result)
                        });
                    }
                }
            }

            _ = poll.tick() => {
                let Some(r) = &mut running else { continue };

                if r.core.client.is_closed() {
                    if let Some(r) = running.take() {
                        r.core.shutdown().await;
                    }
                    let _ = events.send(Event::Status(Status::Disconnected)).await;
                    continue;
                }

                // The core may have crashed on its own, for example because of a TUN failure.
                if let Some(status) = r.core.exited() {
                    let _ = events
                        .send(Event::Status(Status::Failed(format!(
                            "ядро завершилось ({status})"
                        ))))
                        .await;
                    running = None;
                    continue;
                }

                let outcome: Result<()> = async {
                {
                    let stats = control(r.core.client.query_stats()).await.context("статистика ядра")?;
                    let sum = |m: &std::collections::HashMap<String, i64>| -> i64 {
                        r.tags.iter().filter_map(|t| m.get(t)).sum()
                    };
                    let (down, up) = (sum(&stats.downs), sum(&stats.ups));
                    if down != 0 || up != 0 {
                        let _ = events.send(Event::Traffic { down, up }).await;
                    }
                }

                if !r.auto_members.is_empty() {
                    {
                        let response = control(r.core.client.query_auto_selectors()).await.context("автовыбор ядра")?;
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

                {
                    let conns = control(r.core.client.query_connections()).await.context("соединения ядра")?;
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
                Ok(())
                }.await;
                if let Err(error) = outcome {
                    if let Some(r) = running.take() {
                        r.core.shutdown().await;
                    }
                    let _ = events.send(Event::Status(Status::Failed(format!("{error:#}")))).await;
                }
            }
        }
    }
    cancel_tests(&mut workers, &events).await;
    cancel_background(&mut subscriptions).await;
    if let Some(r) = running {
        r.core.shutdown().await;
    }
}

async fn cancel_background<T: Send + 'static>(tasks: &mut tokio::task::JoinSet<T>) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn require_vpn_privilege(core: &Core, core_bin: &Path) -> Result<()> {
    if !control(core.client.is_privileged())
        .await
        .context("проверка прав ядра VPN")?
    {
        anyhow::bail!(vpn_privilege_hint(core_bin));
    }
    Ok(())
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
    core_bin: &Path,
    runtime_dir: &Path,
    profile: &Profile,
    settings: &Settings,
) -> Result<Running> {
    let generated = throne_config::generate(profile, settings).context("сборка конфига")?;
    let core = if settings.mode.is_vpn() {
        Core::spawn_privileged(core_bin, runtime_dir, settings.log_level == "debug").await?
    } else {
        Core::spawn(core_bin, runtime_dir, settings.log_level == "debug").await?
    };

    if settings.mode.is_vpn() {
        if let Err(error) = require_vpn_privilege(&core, core_bin).await {
            core.shutdown().await;
            return Err(error);
        }
    }

    let outcome = core
        .client
        .start(load_request(&generated, settings))
        .await
        .context("запуск конфига в ядре");
    if let Err(error) = outcome {
        core.shutdown().await;
        return Err(error);
    }

    Ok(Running {
        core,
        profile_id: profile.id,
        tags: generated.tags,
        auto_members: Vec::new(),
    })
}

/// Connect with auto-selection. Profiles are already sorted: the core treats
/// their order as the initial ranking.
async fn connect_auto(
    core_bin: &Path,
    runtime_dir: &Path,
    profiles: &[Profile],
    gid: i64,
    settings: &Settings,
) -> Result<Running> {
    if profiles.is_empty() {
        anyhow::bail!("в группе нет серверов");
    }
    let generated =
        throne_config::generate::generate_auto(profiles, settings).context("сборка конфига")?;
    let core = if settings.mode.is_vpn() {
        Core::spawn_privileged(core_bin, runtime_dir, settings.log_level == "debug").await?
    } else {
        Core::spawn(core_bin, runtime_dir, settings.log_level == "debug").await?
    };

    if settings.mode.is_vpn() {
        if let Err(error) = require_vpn_privilege(&core, core_bin).await {
            core.shutdown().await;
            return Err(error);
        }
    }

    let outcome = core
        .client
        .start(load_request(&generated, settings))
        .await
        .context("запуск конфига в ядре");
    if let Err(error) = outcome {
        core.shutdown().await;
        return Err(error);
    }

    let auto_members: Vec<(String, String)> = generated
        .tags
        .iter()
        .cloned()
        .zip(profiles.iter().map(|p| p.name.clone()))
        .collect();

    // The core counts traffic on the outbound that actually connected, not on
    // the group: both the selector and all its members must be summed.
    let mut tags = vec![throne_config::generate::tags::PROXY.to_string()];
    tags.extend(auto_members.iter().map(|(tag, _)| tag.clone()));

    Ok(Running {
        core,
        profile_id: -gid,
        tags,
        auto_members,
    })
}

fn vpn_privilege_hint(_core_bin: &Path) -> String {
    #[cfg(target_os = "linux")]
    return format!(
        "для режима VPN ядру нужны права на создание TUN-интерфейса.\n\
         Выдайте их один раз: sudo setcap cap_net_admin,cap_net_raw+ep {}",
        _core_bin.display()
    );

    #[cfg(target_os = "windows")]
    return "для режима VPN перезапустите Throne GTK от имени администратора".into();

    #[cfg(target_os = "macos")]
    return "root-helper запущен, но не получил права на создание TUN-интерфейса".into();

    #[allow(unreachable_code)]
    "для режима VPN нужны права на создание TUN-интерфейса".into()
}

async fn check_config(
    core_bin: &Path,
    runtime_dir: &Path,
    profile: &Profile,
    settings: &Settings,
) -> Result<()> {
    let generated = throne_config::generate(profile, settings).context("сборка конфига")?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;
    let outcome = core
        .client
        .check_config(load_request(&generated, settings))
        .await;
    core.shutdown().await;
    outcome
}

async fn test_latency(
    core_bin: &Path,
    runtime_dir: &Path,
    profiles: Vec<Profile>,
    ids: Vec<i64>,
    settings: &Settings,
    events: &TestEvents,
) -> Result<()> {
    if profiles.is_empty() {
        let _ = events.send(Event::TestFinished { tested: 0 }).await;
        return Ok(());
    }
    let generated =
        throne_config::generate_test(&profiles, settings).context("сборка конфига теста")?;
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
            // A tag of the form `p-<index>` binds the result to a list row.
            let Some(index) = generated
                .tags
                .iter()
                .position(|t| t == result.outbound_tag())
            else {
                continue;
            };
            let Some(&id) = ids.get(index) else { continue };
            // Encode a failed test as negative latency: zero means “not checked
            // yet,” and these two states must not be confused.
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
    if outcome.is_ok() {
        let _ = events.send(Event::TestFinished { tested }).await;
    }
    outcome.map(|_| ())
}

/// Speed measurement. The core performs it; our task is to build the config,
/// poll the state while it runs, and return the result as one record.
async fn speed_test(
    core_bin: &Path,
    runtime_dir: &Path,
    profile: Profile,
    id: i64,
    settings: &Settings,
    events: &TestEvents,
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

    // While the core is measuring, query it for progress.
    let progress = async {
        let client = &core.client;
        let mut tick = tokio::time::interval(Duration::from_millis(700));
        loop {
            tick.tick().await;
            let Ok(state) = client.query_speed_test().await else {
                break;
            };
            if !state.is_running() {
                continue;
            }
            let Some(result) = &state.result else {
                continue;
            };
            let stage = if !result.dl_speed().is_empty() {
                format!("приём {}", result.dl_speed())
            } else if !result.server_name().is_empty() {
                format!("сервер {}", result.server_name())
            } else {
                "замеряю…".to_string()
            };
            let _ = events.send(Event::SpeedProgress { id, stage }).await;
        }
    };

    // Progress is a child future, not a detached task: dropping/aborting this
    // worker drops both the request and its progress poller before the core.
    let outcome = tokio::select! {
        outcome = core.client.speed_test(request) => outcome,
        _ = progress => Err(anyhow::anyhow!("канал замера закрыт")),
    };
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

async fn fetch_subscription(
    url: &str,
    user_agent: &str,
    send_hwid: bool,
    custom_hwid_params: &str,
) -> Result<subscription::Parsed> {
    let (body, userinfo) =
        subscription::fetch(url, user_agent, 30, send_hwid, custom_hwid_params).await?;
    let mut parsed = subscription::parse(&body)?;
    if !userinfo.is_empty() {
        parsed.info = subscription::format_userinfo(&userinfo);
    }
    Ok(parsed)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn background_cancellation_joins_and_drops_network_work() {
        let flag = Arc::new(AtomicBool::new(false));
        let guard = Dropped(flag.clone());
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        cancel_background(&mut tasks).await;
        assert!(tasks.is_empty());
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_awaits_workers_and_completes_each_type_once() {
        let (events, receiver) = async_channel::unbounded();
        let mut workers = HashMap::new();
        let mut dropped = Vec::new();
        for (generation, key) in [(1, TestKey::Latency), (2, TestKey::Speed(7))] {
            let flag = Arc::new(AtomicBool::new(false));
            let guard = Dropped(flag.clone());
            let task = tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            workers.insert(key, TestWorker { generation, task });
            dropped.push(flag);
        }
        cancel_tests(&mut workers, &events).await;
        assert!(workers.is_empty());
        assert!(dropped.iter().all(|flag| flag.load(Ordering::SeqCst)));
        assert_eq!(receiver.len(), 2);
        cancel_tests(&mut workers, &events).await;
        assert_eq!(receiver.len(), 2);
    }

    #[tokio::test]
    async fn cancelled_generation_cannot_forward_queued_results() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let (events, _) = async_channel::unbounded();
        let task = tokio::spawn(std::future::pending());
        let mut workers = HashMap::from([(
            TestKey::Latency,
            TestWorker {
                generation: 1,
                task,
            },
        )]);
        let old_events = TestEvents {
            generation: 1,
            sender,
        };
        old_events
            .send(Event::LatencyResult {
                id: 9,
                latency: 100,
            })
            .await
            .unwrap();
        cancel_tests(&mut workers, &events).await;
        let (generation, _) = receiver.recv().await.unwrap();
        assert!(!workers
            .values()
            .any(|worker| worker.generation == generation));
    }
}
