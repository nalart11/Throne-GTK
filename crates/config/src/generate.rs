//! Сборка конфига для ядра.
//!
//! Профили хранятся как outbound-ы sing-box, поэтому генератор в основном
//! обвязка: вход (mixed/TUN), DNS, маршруты. Отдельная работа — профили на
//! XHTTP: этот транспорт умеет только Xray, поэтому для них поднимается
//! локальный socks-мост, в который sing-box ходит как в обычный прокси.

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

use crate::profile::Profile;
use crate::settings::Settings;
use crate::xray;

pub mod tags {
    pub const PROXY: &str = "proxy";
    pub const DIRECT: &str = "direct";
    pub const MIXED_IN: &str = "mixed-in";
    pub const TUN_IN: &str = "tun-in";
    pub const DNS_REMOTE: &str = "dns-remote";
    pub const DNS_DIRECT: &str = "dns-direct";
    pub const DNS_LOCAL: &str = "dns-local";
}

/// Готовый к отправке в ядро набор конфигов.
#[derive(Debug, Clone, Default)]
pub struct GeneratedConfig {
    pub core_config: String,
    pub xray_config: String,
    pub need_xray: bool,
    pub tun_ipv4_cidr: String,
    /// Теги outbound-ов в том же порядке, в каком пришли профили: по ним
    /// раскладываются результаты тестов и статистика.
    pub tags: Vec<String>,
}

/// Зачем собирается конфиг: от этого зависят входы, DNS и то, один ли
/// outbound окажется под тегом `proxy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// Работа с одним выбранным сервером.
    Single,
    /// Пакетная проверка: без входов, каждый сервер под своим тегом.
    Test,
    /// Работа с автовыбором: под тегом `proxy` стоит селектор, а серверы —
    /// его участники.
    Auto,
}

/// Рабочий конфиг для одного выбранного профиля.
pub fn generate(profile: &Profile, settings: &Settings) -> Result<GeneratedConfig> {
    build(std::slice::from_ref(profile), settings, Purpose::Single)
}

/// Конфиг для пакетного теста: без входов, все профили сразу.
pub fn generate_test(profiles: &[Profile], settings: &Settings) -> Result<GeneratedConfig> {
    build(profiles, settings, Purpose::Test)
}

/// Рабочий конфиг с автовыбором сервера из группы.
pub fn generate_auto(profiles: &[Profile], settings: &Settings) -> Result<GeneratedConfig> {
    build(profiles, settings, Purpose::Auto)
}

fn build(profiles: &[Profile], settings: &Settings, purpose: Purpose) -> Result<GeneratedConfig> {
    let for_test = purpose == Purpose::Test;
    if profiles.is_empty() {
        bail!("не выбрано ни одного профиля");
    }

    let mut result = GeneratedConfig {
        tun_ipv4_cidr: settings.tun_ipv4_cidr.clone(),
        ..Default::default()
    };
    let mut outbounds: Vec<Value> = Vec::with_capacity(profiles.len() + 1);
    let mut xray_bridges: Vec<xray::Bridge> = Vec::new();

    for (index, profile) in profiles.iter().enumerate() {
        // При одном сервере он и есть `proxy`; в тесте и при автовыборе теги
        // должны быть различимы, поэтому нумеруем.
        let tag = match purpose {
            Purpose::Single => tags::PROXY.to_string(),
            Purpose::Test | Purpose::Auto => format!("p-{index}"),
        };

        if profile.is_xray() {
            let bridge = xray::Bridge::reserve(profile, &tag)?;
            outbounds.push(bridge.socks_outbound());
            xray_bridges.push(bridge);
        } else {
            outbounds.push(apply_mux(profile.tagged(&tag), settings));
        }
        result.tags.push(tag);
    }

    if purpose == Purpose::Auto {
        outbounds.push(auto_selector(&result.tags, profiles, settings));
    }

    outbounds.push(json!({"type": "direct", "tag": tags::DIRECT}));

    let mut config = Map::new();
    config.insert(
        "log".into(),
        json!({"level": settings.log_level, "timestamp": true}),
    );
    // В тестовом конфиге своей службы DNS нет: её `dns-remote` ходит через
    // outbound `proxy`, которого в тесте не существует, и ядро отказывается
    // запускать сервисы. Имена серверов резолвит система.
    if settings.dns_enabled && !for_test {
        config.insert("dns".into(), dns_section(settings));
    }
    if !for_test {
        config.insert("inbounds".into(), inbounds_section(settings));
    }
    config.insert("outbounds".into(), Value::Array(outbounds));
    config.insert("route".into(), route_section(settings, for_test)?);
    if !for_test {
        // store_rdrc объявлен устаревшим в sing-box 1.14, а выгоды от него
        // на десктопе нет: правила перечитываются за миллисекунды.
        let mut cache = json!({
            "enabled": true,
            "store_fakeip": settings.fakedns,
        });
        if !settings.cache_file.is_empty() {
            cache["path"] = json!(settings.cache_file);
        }
        let mut experimental = json!({"cache_file": cache});
        if settings.stats_enabled {
            // Счётчики трафика ядро ведёт только при наличии clash_api;
            // пустой default_mode нужен, чтобы объект вообще появился, а
            // слушателя без external_controller ядро не поднимает.
            experimental["clash_api"] = json!({"default_mode": ""});
        }
        config.insert("experimental".into(), experimental);
    }

    if !xray_bridges.is_empty() {
        result.need_xray = true;
        result.xray_config = xray::build_config(&xray_bridges, settings)?;
    }
    result.core_config = serde_json::to_string_pretty(&Value::Object(config))?;
    Ok(result)
}

/// Мультиплексирование поверх протоколов, которые его поддерживают. У QUIC-овых
/// (hysteria2, tuic) свои потоки, им mux только мешает.
fn apply_mux(mut outbound: Value, settings: &Settings) -> Value {
    if !settings.mux_enabled {
        return outbound;
    }
    let kind = outbound.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "vless" | "vmess" | "trojan" | "shadowsocks") {
        return outbound;
    }
    // vision несёт собственное мультиплексирование, поверх него mux запрещён.
    if outbound.get("flow").and_then(Value::as_str).is_some_and(|f| !f.is_empty()) {
        return outbound;
    }
    if let Some(obj) = outbound.as_object_mut() {
        obj.insert(
            "multiplex".into(),
            json!({
                "enabled": true,
                "protocol": settings.mux_protocol,
                "max_streams": settings.mux_max_streams,
            }),
        );
    }
    outbound
}

/// Группа автовыбора. Порядок участников — исходное ранжирование, поэтому
/// профили передаются уже отсортированными по задержке; кроме того, известные
/// задержки отдаются как «тёплое» состояние, и первый же выбор оказывается
/// осмысленным, а не случайным.
fn auto_selector(member_tags: &[String], profiles: &[Profile], settings: &Settings) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let warm: Vec<Value> = member_tags
        .iter()
        .zip(profiles)
        .filter_map(|(tag, profile)| {
            // 0 — «не проверяли»; отрицательное — «не отвечал», и это ядро
            // тоже должно знать, поэтому пишем rtt = 0.
            if profile.latency == 0 || profile.latency_at == 0 {
                return None;
            }
            let rtt = profile.latency.max(0).min(u16::MAX as i32) as u16;
            let age = (now - profile.latency_at).max(0) as u64;
            Some(json!({"tag": tag, "rtt": rtt, "age": age}))
        })
        .collect();

    let mut selector = json!({
        "type": "auto-selector",
        "tag": tags::PROXY,
        "outbounds": member_tags,
        "url": settings.test_url,
        "interval": format!("{}s", settings.auto_interval_secs.max(30)),
        "tolerance": settings.auto_tolerance_ms.max(0),
        "timeout": format!("{}ms", settings.test_timeout_ms.max(1000)),
        "concurrency": settings.test_concurrency.max(1),
        // Осечка выбранного сервера не должна доходить до приложения, пока в
        // группе есть живые: пробуем следующих по ранжированию.
        "dial_retries": 2,
    });
    if !warm.is_empty() {
        selector["warm"] = Value::Array(warm);
    }
    if settings.auto_balance {
        selector["balance"] = json!(true);
        selector["balance_mode"] = json!("rotate");
    }
    selector
}

fn inbounds_section(settings: &Settings) -> Value {
    let mut inbounds = Vec::new();

    let listen = if settings.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    inbounds.push(json!({
        "type": "mixed",
        "tag": tags::MIXED_IN,
        "listen": listen,
        "listen_port": settings.mixed_port,
    }));

    if settings.mode.is_vpn() {
        let mut address = vec![settings.tun_ipv4_cidr.clone()];
        if settings.tun_ipv6 {
            address.push(settings.tun_ipv6_cidr.clone());
        }
        inbounds.push(json!({
            "type": "tun",
            "tag": tags::TUN_IN,
            "address": address,
            "mtu": settings.tun_mtu,
            "auto_route": true,
            "strict_route": settings.tun_strict_route,
            "stack": settings.tun_stack.as_str(),
        }));
    }

    Value::Array(inbounds)
}

/// Адрес резолвера в объект сервера sing-box 1.13: `tls://1.1.1.1` →
/// `{"type": "tls", "server": "1.1.1.1"}`.
fn dns_server(address: &str) -> Value {
    let address = address.trim();
    if address.is_empty() || address == "local" {
        return json!({"type": "local"});
    }
    if let Some(iface) = address.strip_prefix("dhcp://") {
        return json!({"type": "dhcp", "interface": if iface == "auto" { "" } else { iface }});
    }

    let (kind, rest) = match address.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("udp", address),
    };

    match kind {
        "https" | "h3" => {
            let (host, path) = match rest.split_once('/') {
                Some((h, p)) => (h, format!("/{p}")),
                None => (rest, "/dns-query".to_string()),
            };
            let (server, port) = split_optional_port(host);
            let mut o = json!({"type": kind, "server": server, "path": path});
            if let Some(port) = port {
                o["server_port"] = json!(port);
            }
            o
        }
        "tcp" | "tls" | "quic" | "udp" => {
            let (server, port) = split_optional_port(rest);
            let mut o = json!({"type": kind, "server": server});
            if let Some(port) = port {
                o["server_port"] = json!(port);
            }
            o
        }
        _ => json!({"type": "udp", "server": rest}),
    }
}

fn split_optional_port(s: &str) -> (String, Option<u16>) {
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail.strip_prefix(':').and_then(|p| p.parse().ok());
            return (host.to_string(), port);
        }
    }
    // Голый IPv6 без скобок: двоеточий много, порта нет.
    if s.matches(':').count() > 1 {
        return (s.to_string(), None);
    }
    match s.split_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().ok()),
        None => (s.to_string(), None),
    }
}

fn dns_section(settings: &Settings) -> Value {
    let mut remote = dns_server(&settings.dns_remote);
    remote["tag"] = json!(tags::DNS_REMOTE);
    remote["detour"] = json!(tags::PROXY);
    // Имя самого DNS-сервера резолвится локально — иначе получаем курицу и яйцо.
    remote["domain_resolver"] = json!(tags::DNS_LOCAL);

    let mut direct = dns_server(&settings.dns_direct);
    direct["tag"] = json!(tags::DNS_DIRECT);
    // detour на пустой direct-outbound sing-box считает ошибкой и не стартует:
    // запросы этого сервера и так идут напрямую.
    direct["domain_resolver"] = json!(tags::DNS_LOCAL);

    let servers = vec![remote, direct, json!({"type": "local", "tag": tags::DNS_LOCAL})];

    let mut rules = Vec::new();
    if settings.fakedns {
        rules.push(json!({
            "query_type": ["A", "AAAA"],
            "action": "route",
            "server": "dns-fake",
        }));
    }
    // Всё, что уходит напрямую, и резолвится напрямую.
    if settings.dns_routing && settings.bypass_private {
        rules.push(json!({
            "action": "route",
            "server": tags::DNS_DIRECT,
            "rule_set": ["geosite-private"],
        }));
    }
    rules.push(json!({
        "action": "route",
        "server": tags::DNS_REMOTE,
        "strategy": settings.dns_strategy.as_str(),
    }));

    let mut dns = json!({
        "servers": servers,
        "rules": rules,
    });
    // Раздельный кэш нужен только когда часть ответов подставная: иначе это
    // лишняя память, и в sing-box 1.14 опция объявлена устаревшей.
    if settings.fakedns {
        dns["independent_cache"] = json!(true);
    }
    if settings.fakedns {
        dns["servers"].as_array_mut().unwrap().push(json!({
            "tag": "dns-fake",
            "type": "fakeip",
            "inet4_range": "198.18.0.0/15",
            "inet6_range": "fc00::/18",
        }));
    }
    dns
}

fn route_section(settings: &Settings, for_test: bool) -> Result<Value> {
    let mut rules: Vec<Value> = Vec::new();

    if !for_test {
        if settings.sniffing {
            rules.push(json!({"action": "sniff"}));
        }
        rules.push(json!({"protocol": "dns", "action": "hijack-dns"}));
        if settings.dns_strategy.as_str() != "" {
            rules.push(json!({
                "inbound": [tags::MIXED_IN, tags::TUN_IN],
                "action": "resolve",
                "strategy": settings.dns_strategy.as_str(),
            }));
        }
    }

    // Правила из интерфейса идут первыми: они выражают намерение человека и
    // должны иметь приоритет над всем, что добавляется автоматически.
    let mut rule_sets: Vec<Value> = Vec::new();
    let mut declared: std::collections::BTreeSet<String> = Default::default();
    for rule in &settings.route_rules {
        if let Some(value) = rule.to_singbox() {
            rules.push(value);
        }
        for name in rule.rule_sets() {
            if declared.insert(name.clone()) {
                if let Some(declaration) = crate::route::rule_set_declaration(&name) {
                    rule_sets.push(declaration);
                }
            }
        }
    }

    if !settings.custom_route_rules.trim().is_empty() {
        let custom: Value = serde_json::from_str(settings.custom_route_rules.trim())
            .map_err(|e| anyhow::anyhow!("свои правила маршрутизации — не JSON: {e}"))?;
        match custom {
            Value::Array(items) => rules.extend(items),
            Value::Object(_) => rules.push(custom),
            _ => bail!("свои правила маршрутизации должны быть объектом или массивом"),
        }
    }

    if settings.bypass_private {
        rules.push(json!({
            "action": "route",
            "outbound": tags::DIRECT,
            "ip_is_private": true,
        }));
        rules.push(json!({
            "action": "route",
            "outbound": tags::DIRECT,
            "rule_set": ["geosite-private"],
        }));
        // Через ту же обвязку, что и списки из правил: адрес зеркала должен
        // быть один на весь конфиг.
        if let Some(declaration) = crate::route::rule_set_declaration("geosite-private") {
            rule_sets.push(declaration);
        }
    }

    let mut route = json!({
        "rules": rules,
        "final": if for_test { tags::DIRECT } else { tags::PROXY },
    });
    // Резолвер по умолчанию указывает на нашу службу DNS; в тесте её нет.
    if !for_test {
        route["default_domain_resolver"] = json!({
            "server": tags::DNS_DIRECT,
            "strategy": settings.dns_strategy.as_str(),
        });
    }
    if !rule_sets.is_empty() {
        route["rule_set"] = Value::Array(rule_sets);
    }
    if !for_test {
        if settings.stats_enabled {
            route["find_process"] = json!(true);
        }
        if settings.mode.is_vpn() {
            route["auto_detect_interface"] = json!(true);
        }
    }
    Ok(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link;

    fn vless() -> Profile {
        link::parse("vless://uuid@a.example:443?security=reality&pbk=KEY&sid=00&flow=xtls-rprx-vision#a")
            .unwrap()
    }

    fn parse_config(g: &GeneratedConfig) -> Value {
        serde_json::from_str(&g.core_config).unwrap()
    }

    #[test]
    fn working_config_has_one_proxy_and_inbounds() {
        let g = generate(&vless(), &Settings::default()).unwrap();
        let v = parse_config(&g);
        assert_eq!(g.tags, vec!["proxy"]);
        assert_eq!(v["outbounds"][0]["tag"], "proxy");
        assert_eq!(v["outbounds"][0]["type"], "vless");
        assert_eq!(v["outbounds"][1]["tag"], "direct");
        assert_eq!(v["inbounds"][0]["type"], "mixed");
        assert_eq!(v["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(v["inbounds"][0]["listen_port"], 2080);
        assert_eq!(v["route"]["final"], "proxy");
        assert!(!g.need_xray);
    }

    #[test]
    fn vpn_mode_adds_tun_and_auto_detect() {
        let mut s = Settings::default();
        s.mode = crate::settings::ProxyMode::Vpn;
        let v = parse_config(&generate(&vless(), &s).unwrap());
        let tun = &v["inbounds"][1];
        assert_eq!(tun["type"], "tun");
        assert_eq!(tun["address"][0], "172.19.0.1/30");
        assert_eq!(tun["stack"], "mixed");
        assert_eq!(tun["auto_route"], true);
        assert_eq!(v["route"]["auto_detect_interface"], true);
    }

    /// Без clash_api ядро не ведёт счётчики, и график трафика остаётся пустым.
    #[test]
    fn stats_require_clash_api() {
        let mut s = Settings::default();
        let v = parse_config(&generate(&vless(), &s).unwrap());
        assert!(v["experimental"]["clash_api"].is_object());

        s.stats_enabled = false;
        let v = parse_config(&generate(&vless(), &s).unwrap());
        assert!(v["experimental"].get("clash_api").is_none());
    }

    #[test]
    fn cache_file_path_is_used_when_set() {
        let mut s = Settings::default();
        s.cache_file = "/tmp/x/cache.db".into();
        let v = parse_config(&generate(&vless(), &s).unwrap());
        assert_eq!(v["experimental"]["cache_file"]["path"], "/tmp/x/cache.db");
    }

    #[test]
    fn allow_lan_opens_the_listener() {
        let mut s = Settings::default();
        s.allow_lan = true;
        let v = parse_config(&generate(&vless(), &s).unwrap());
        assert_eq!(v["inbounds"][0]["listen"], "0.0.0.0");
    }

    /// В тестовом конфиге не должно остаться ссылок на outbound `proxy`:
    /// его там нет, и ядро отказывается запускать с ним сервисы.
    #[test]
    fn auto_config_puts_selector_under_proxy_tag() {
        let mut a = vless();
        a.name = "a".into();
        a.latency = 40;
        a.latency_at = 1_700_000_000;
        let mut b = vless();
        b.name = "b".into();

        let g = generate_auto(&[a, b], &Settings::default()).unwrap();
        let v: Value = serde_json::from_str(&g.core_config).unwrap();
        assert_eq!(g.tags, vec!["p-0", "p-1"]);

        let outbounds = v["outbounds"].as_array().unwrap();
        let selector = outbounds.iter().find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(selector["type"], "auto-selector");
        assert_eq!(selector["outbounds"][0], "p-0");
        assert_eq!(selector["outbounds"][1], "p-1");
        // Известная задержка передаётся селектору, неизвестная — нет.
        assert_eq!(selector["warm"].as_array().unwrap().len(), 1);
        assert_eq!(selector["warm"][0]["rtt"], 40);

        // Вход и маршрутизация остаются как при обычном подключении.
        assert_eq!(v["inbounds"][0]["type"], "mixed");
        assert_eq!(v["route"]["final"], "proxy");
    }

    #[test]
    fn test_config_has_no_inbounds_and_numbered_tags() {
        let profiles = vec![vless(), vless()];
        let g = generate_test(&profiles, &Settings::default()).unwrap();
        let v = parse_config(&g);
        assert_eq!(g.tags, vec!["p-0", "p-1"]);
        assert!(v.get("inbounds").is_none());
        assert!(v.get("dns").is_none());
        assert!(v["route"].get("default_domain_resolver").is_none());
        assert_eq!(v["route"]["final"], "direct");
        assert!(!g.core_config.contains("\"proxy\""), "тестовый конфиг ссылается на proxy");
    }

    #[test]
    fn dns_addresses_become_typed_servers() {
        assert_eq!(dns_server("tls://1.1.1.1")["type"], "tls");
        assert_eq!(dns_server("tls://1.1.1.1")["server"], "1.1.1.1");
        assert_eq!(dns_server("8.8.8.8")["type"], "udp");
        assert_eq!(dns_server("https://dns.google/dns-query")["path"], "/dns-query");
        assert_eq!(dns_server("udp://[2606:4700::1111]:53")["server"], "2606:4700::1111");
        assert_eq!(dns_server("udp://[2606:4700::1111]:53")["server_port"], 53);
        assert_eq!(dns_server("local")["type"], "local");
    }

    #[test]
    fn remote_dns_goes_through_the_proxy() {
        let v = parse_config(&generate(&vless(), &Settings::default()).unwrap());
        let servers = v["dns"]["servers"].as_array().unwrap();
        let remote = servers.iter().find(|s| s["tag"] == "dns-remote").unwrap();
        assert_eq!(remote["detour"], "proxy");
        assert_eq!(remote["domain_resolver"], "dns-local");
    }

    /// sing-box отказывается стартовать, если у сервера прописан detour на
    /// пустой direct-outbound. Проверка конфига этого не ловит — только старт.
    #[test]
    fn direct_dns_has_no_detour() {
        let v = parse_config(&generate(&vless(), &Settings::default()).unwrap());
        let servers = v["dns"]["servers"].as_array().unwrap();
        let direct = servers.iter().find(|s| s["tag"] == "dns-direct").unwrap();
        assert!(direct.get("detour").is_none());
    }

    #[test]
    fn mux_skips_vision_and_quic() {
        let mut s = Settings::default();
        s.mux_enabled = true;
        // vision — mux запрещён
        let v = parse_config(&generate(&vless(), &s).unwrap());
        assert!(v["outbounds"][0].get("multiplex").is_none());

        // hysteria2 — свои потоки
        let hy = link::parse("hy2://pw@h.example:443#h").unwrap();
        let v = parse_config(&generate(&hy, &s).unwrap());
        assert!(v["outbounds"][0].get("multiplex").is_none());

        // обычный vmess — mux применяется
        let vm = link::parse("trojan://pw@t.example:443#t").unwrap();
        let v = parse_config(&generate(&vm, &s).unwrap());
        assert_eq!(v["outbounds"][0]["multiplex"]["enabled"], true);
    }

    #[test]
    fn custom_rules_are_inserted_before_generated_ones() {
        let mut s = Settings::default();
        s.custom_route_rules = r#"[{"domain_suffix": [".local"], "action": "route", "outbound": "direct"}]"#.into();
        let v = parse_config(&generate(&vless(), &s).unwrap());
        let rules = v["route"]["rules"].as_array().unwrap();
        let idx = rules
            .iter()
            .position(|r| r["domain_suffix"][0] == ".local")
            .unwrap();
        let private = rules.iter().position(|r| r["ip_is_private"] == true).unwrap();
        assert!(idx < private);
    }

    #[test]
    fn interface_rules_come_first_and_declare_their_lists() {
        use crate::route::{MatchKind, RouteAction, RouteRule};

        let mut s = Settings::default();
        s.route_rules = vec![
            RouteRule {
                name: "Россия напрямую".into(),
                ..RouteRule::single(
                    MatchKind::RuleSet,
                    vec!["geosite-ru".into()],
                    RouteAction::Direct,
                )
            },
            RouteRule {
                name: "Реклама".into(),
                ..RouteRule::single(
                    MatchKind::DomainKeyword,
                    vec!["ads".into()],
                    RouteAction::Block,
                )
            },
        ];
        s.custom_route_rules = r#"[{"domain": ["x.example"], "action": "route", "outbound": "direct"}]"#.into();

        let v = parse_config(&generate(&vless(), &s).unwrap());
        let rules = v["route"]["rules"].as_array().unwrap();
        let interface = rules.iter().position(|r| r["rule_set"][0] == "geosite-ru").unwrap();
        let custom = rules.iter().position(|r| r["domain"][0] == "x.example").unwrap();
        let private = rules.iter().position(|r| r["ip_is_private"] == true).unwrap();
        assert!(interface < custom && custom < private);

        assert!(rules.iter().any(|r| r["action"] == "reject"));

        // Список, на который ссылается правило, обязан быть объявлен.
        let sets = v["route"]["rule_set"].as_array().unwrap();
        assert!(sets.iter().any(|s| s["tag"] == "geosite-ru"));
    }

    #[test]
    fn broken_custom_rules_are_reported() {
        let mut s = Settings::default();
        s.custom_route_rules = "{ это не json }".into();
        assert!(generate(&vless(), &s).is_err());
    }

    #[test]
    fn empty_selection_is_an_error() {
        assert!(generate_test(&[], &Settings::default()).is_err());
    }
}
