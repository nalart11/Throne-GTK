//! Проверка сквозного пути на настоящих данных: импорт базы Throne →
//! генерация конфига → проверка этого конфига ядром.
//!
//! Запуск: `just selftest` — рецепт кладёт этот бинарь и ядро в одну папку под
//! именем `throne-gtk`, потому что ядро отказывается работать с любым другим
//! родителем. Ничего не меняет: база импортируется в память, ядро только
//! валидирует конфиг и сразу гасится.

use std::path::PathBuf;

use anyhow::Result;
use throne_config::Settings;
use throne_config::generate::GeneratedConfig;
use throne_ipc::Core;
use throne_store::Store;

fn load_request(generated: &GeneratedConfig) -> throne_ipc::pb::LoadConfigReq {
    let mut request = throne_ipc::load_request(generated.core_config.clone());
    request.need_xray = Some(generated.need_xray);
    request.xray_config = Some(generated.xray_config.clone());
    request
}

#[tokio::main]
async fn main() -> Result<()> {
    let throne_db = PathBuf::from(std::env::var("HOME")?).join(".config/Throne/config/throne.db");
    if !throne_db.exists() {
        println!("базы Throne нет ({}) — пропускаю", throne_db.display());
        return Ok(());
    }

    // `--seed <файл>` пишет импорт в настоящую базу: тот же перенос, что и
    // кнопкой в меню, но из терминала — пригодится, если интерфейс не
    // поднимается.
    let seed = std::env::args()
        .skip_while(|a| a != "--seed")
        .nth(1)
        .map(PathBuf::from);

    let mut store = match &seed {
        Some(path) => Store::open(path)?,
        None => Store::open_memory()?,
    };
    let done = store.import_throne(&throne_db)?;
    println!(
        "импортировано: групп {}, серверов {}, правил {} (пропущено правил {})",
        done.groups, done.profiles, done.rules, done.skipped_rules
    );

    if let Some(path) = &seed {
        println!("записано в {}", path.display());
    }

    let all = store.profiles(None)?;
    let settings = Settings::default();

    // Сначала — что вообще собирается в конфиг, без запуска ядра.
    let mut by_kind: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for profile in &all {
        let entry = by_kind.entry(profile.kind.clone()).or_default();
        entry.0 += 1;
        if throne_config::generate(profile, &settings).is_ok() {
            entry.1 += 1;
        }
    }
    println!("\nпрофили по протоколам (всего / собралось в конфиг):");
    for (kind, (total, ok)) in &by_kind {
        println!("  {kind:<12} {ok}/{total}");
    }

    // Затем — что из собранного примет само ядро.
    // Ядро ищем рядом с собой: только так проходит его проверка родителя.
    let core_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("throne-gtk-core")))
        .filter(|p| p.exists());
    let Some(core_bin) = core_bin else {
        println!("\nядро не собрано — проверку конфигов ядром пропускаю");
        return Ok(());
    };

    let runtime_dir = std::env::temp_dir().join("throne-gtk-selftest");
    let mut checked = 0;
    let mut failed = Vec::new();

    // Ядро проверяет по одному профилю на запуск, поэтому берём выборку:
    // по первому профилю каждого протокола.
    let mut seen = std::collections::BTreeSet::new();
    for profile in &all {
        if !seen.insert(profile.kind.clone()) {
            continue;
        }
        let Ok(generated) = throne_config::generate(profile, &settings) else {
            continue;
        };
        let core = Core::spawn(&core_bin, &runtime_dir, false).await?;
        let outcome = core
            .client
            .check_config(load_request(&generated))
            .await;
        core.shutdown().await;

        checked += 1;
        match outcome {
            Ok(()) => println!("  {:<12} конфиг принят", profile.kind),
            Err(e) => {
                println!("  {:<12} ОТКАЗ: {e}", profile.kind);
                failed.push(profile.kind.clone());
            }
        }
    }

    println!("\nпроверено ядром: {checked}, отказов: {}", failed.len());
    if !failed.is_empty() {
        anyhow::bail!("ядро не приняло конфиги: {}", failed.join(", "));
    }

    check_synthetic_protocols(&core_bin, &runtime_dir).await?;
    check_auto_selector(&core_bin, &runtime_dir, &all).await?;

    if std::env::args().any(|a| a == "--connect") {
        live_check(&core_bin, &runtime_dir, &all).await?;
    } else {
        println!("\nдля живой проверки соединения добавьте --connect");
    }

    if std::env::args().any(|a| a == "--speed") {
        speed_check(&core_bin, &runtime_dir, &all).await?;
    }

    if std::env::args().any(|a| a == "--latency") {
        latency_check(&core_bin, &runtime_dir, &all).await?;
    }
    Ok(())
}

/// Пакетная проверка задержек — тот же путь, что у кнопки в списке.
async fn latency_check(
    core_bin: &std::path::Path,
    runtime_dir: &std::path::Path,
    profiles: &[throne_config::Profile],
) -> Result<()> {
    let batch: Vec<_> = profiles.iter().take(8).cloned().collect();
    if batch.is_empty() {
        return Ok(());
    }
    println!("\nпроверка задержек ({} серверов):", batch.len());

    let settings = Settings::default();
    let generated = throne_config::generate_test(&batch, &settings)?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;

    let request = throne_ipc::pb::TestReq {
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
    core.shutdown().await;

    let response = outcome?;
    for result in &response.results {
        let index = generated
            .tags
            .iter()
            .position(|t| t == result.outbound_tag())
            .unwrap_or(0);
        let name = batch.get(index).map(|p| p.name.as_str()).unwrap_or("?");
        if result.error().is_empty() {
            println!("  {:>5} мс  {name}", result.latency_ms());
        } else {
            println!("  отказ    {name}: {}", result.error());
        }
    }
    Ok(())
}

/// Конфиг с автовыбором: ядро должно принять группу `auto-selector` со всеми
/// серверами. Это ловит расхождение с форком sing-box, в котором тип живёт.
async fn check_auto_selector(
    core_bin: &std::path::Path,
    runtime_dir: &std::path::Path,
    profiles: &[throne_config::Profile],
) -> Result<()> {
    let group: Vec<_> = profiles.iter().take(12).cloned().collect();
    if group.is_empty() {
        return Ok(());
    }

    let generated = throne_config::generate::generate_auto(&group, &Settings::default())?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;
    let outcome = core.client.check_config(load_request(&generated)).await;
    core.shutdown().await;

    match outcome {
        Ok(()) => println!("\nавтовыбор из {} серверов: конфиг принят", group.len()),
        Err(e) => anyhow::bail!("ядро не приняло конфиг автовыбора: {e}"),
    }
    Ok(())
}

/// Замер скорости первого сервера — тот же путь, что и в интерфейсе.
async fn speed_check(
    core_bin: &std::path::Path,
    runtime_dir: &std::path::Path,
    profiles: &[throne_config::Profile],
) -> Result<()> {
    let Some(profile) = profiles.first() else {
        return Ok(());
    };
    println!("\nзамер скорости: {}", profile.name);

    let mut settings = Settings::default();
    settings.speed_test_upload = true;
    let generated = throne_config::generate_test(std::slice::from_ref(profile), &settings)?;
    let core = Core::spawn(core_bin, runtime_dir, false).await?;

    let mut request = throne_ipc::speed_test_request(generated.core_config.clone());
    request.outbound_tags = generated.tags.clone();
    request.test_download = Some(true);
    request.test_upload = Some(true);
    request.timeout_ms = Some(settings.speed_test_timeout_ms);
    request.need_xray = Some(generated.need_xray);
    request.xray_config = Some(generated.xray_config.clone());

    let outcome = core.client.speed_test(request).await;
    core.shutdown().await;

    match outcome {
        Ok(response) => match response.results.first() {
            Some(result) if result.error().is_empty() => println!(
                "  ↓ {}  ↑ {}  задержка {} мс  сервер {} ({})",
                result.dl_speed(),
                result.ul_speed(),
                result.latency(),
                result.server_name(),
                result.server_country()
            ),
            Some(result) => println!("  ядро вернуло ошибку: {}", result.error()),
            None => println!("  ядро не вернуло результатов"),
        },
        Err(e) => println!("  вызов не прошёл: {e}"),
    }
    Ok(())
}

/// Проверяет, что ядро понимает каждый заявленный протокол — включая те,
/// которых нет в базе. Серверы выдуманные: проверяется разбор конфига, а не
/// связь. Так ловится собранное без нужного тега ядро: тогда ядро отвечает
/// «unknown outbound type».
async fn check_synthetic_protocols(
    core_bin: &std::path::Path,
    runtime_dir: &std::path::Path,
) -> Result<()> {
    const LINKS: &[&str] = &[
        // Ключ reality обязан быть настоящим x25519 в base64url: ядро
        // проверяет его формат ещё при разборе конфига.
        "vless://11111111-1111-1111-1111-111111111111@a.example:443?security=reality\
&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=ab&flow=xtls-rprx-vision#vless",
        "vmess://eyJ2IjoiMiIsInBzIjoidm1lc3MiLCJhZGQiOiJhLmV4YW1wbGUiLCJwb3J0Ijo0NDMsImlkIjoiMTExMTExMTEtMTExMS0xMTExLTExMTEtMTExMTExMTExMTExIiwiYWlkIjowLCJuZXQiOiJ3cyIsInBhdGgiOiIvIiwidGxzIjoidGxzIn0=",
        "trojan://password@a.example:443?sni=a.example#trojan",
        "ss://YWVzLTI1Ni1nY206c2VjcmV0@a.example:8388#shadowsocks",
        "hysteria2://password@a.example:443?obfs=salamander&obfs-password=zzz#hysteria2",
        "tuic://11111111-1111-1111-1111-111111111111:password@a.example:443#tuic",
        "anytls://password@a.example:443?sni=a.example#anytls",
        "naive+https://user:password@a.example:443#naive",
        "naive+quic://user:password@a.example:443?congestion_control=bbr#naive-quic",
        "socks5://user:password@a.example:1080#socks",
        "http://user:password@a.example:8080#http",
    ];

    println!("\nпротоколы на синтетических профилях:");
    let settings = Settings::default();
    let mut failed = Vec::new();

    for link in LINKS {
        let profile = throne_config::link::parse(link)?;
        let generated = throne_config::generate(&profile, &settings)?;
        let core = Core::spawn(core_bin, runtime_dir, false).await?;
        let outcome = core.client.check_config(load_request(&generated)).await;
        core.shutdown().await;

        match outcome {
            Ok(()) => println!("  {:<12} принят", profile.kind),
            Err(e) => {
                println!("  {:<12} ОТКАЗ: {e}", profile.kind);
                failed.push(profile.kind);
            }
        }
    }

    if !failed.is_empty() {
        anyhow::bail!("ядро не знает протоколы: {}", failed.join(", "));
    }
    Ok(())
}

/// Поднимает первый профиль каждого протокола и ходит через него наружу.
/// Порт нестандартный: рядом может работать оригинальный Throne на 2080.
async fn live_check(
    core_bin: &std::path::Path,
    runtime_dir: &std::path::Path,
    profiles: &[throne_config::Profile],
) -> Result<()> {
    println!("\nживая проверка соединения:");
    let mut settings = Settings::default();
    settings.mixed_port = 21080;
    settings.cache_file = std::env::temp_dir()
        .join("throne-gtk-selftest/cache.db")
        .to_string_lossy()
        .into_owned();

    let mut seen = std::collections::BTreeSet::new();
    for profile in profiles {
        if !seen.insert(profile.kind.clone()) {
            continue;
        }
        let generated = throne_config::generate(profile, &settings)?;
        let core = Core::spawn(core_bin, runtime_dir, false).await?;
        let started = core
            .client
            .start(load_request(&generated))
            .await;

        if let Err(e) = started {
            println!("  {:<12} {} — не запустился: {e}", profile.kind, profile.name);
            core.shutdown().await;
            continue;
        }

        // Ядру нужно мгновение, чтобы открыть слушающий порт.
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;

        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("socks5h://127.0.0.1:21080")?)
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let started_at = std::time::Instant::now();
        match client.get("https://www.gstatic.com/generate_204").send().await {
            Ok(response) => println!(
                "  {:<12} {} — {} за {} мс",
                profile.kind,
                profile.name,
                response.status(),
                started_at.elapsed().as_millis()
            ),
            Err(e) => println!("  {:<12} {} — запрос не прошёл: {e}", profile.kind, profile.name),
        }

        let stats = core.client.query_stats().await.ok();
        if let Some(stats) = stats {
            let down: i64 = stats.downs.values().sum();
            let up: i64 = stats.ups.values().sum();
            println!("               трафик: ↓ {down} Б, ↑ {up} Б");
        }
        core.shutdown().await;
    }
    Ok(())
}
