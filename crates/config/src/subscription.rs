//! Загрузка и разбор подписок.
//!
//! Единого формата нет: панели отдают либо base64 от списка ссылок, либо тот же
//! список открытым текстом, либо Clash-YAML, либо готовый JSON sing-box.
//! Определяем формат по содержимому, а не по заголовкам — Content-Type у
//! половины панелей `text/plain` независимо от того, что внутри.

use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use crate::link;
use crate::profile::Profile;

/// Результат разбора: что удалось прочитать и на чём споткнулись.
#[derive(Debug, Default)]
pub struct Parsed {
    pub profiles: Vec<Profile>,
    pub errors: Vec<String>,
    /// Строка вида `упаковано 12 ГиБ из 100 ГиБ, до 2027-02-03` из заголовка
    /// `subscription-userinfo`, если панель его прислала.
    pub info: String,
}

/// Скачивает тело подписки. Возвращает вместе с ним содержимое заголовка
/// `subscription-userinfo` — трафик и срок, которые панели кладут только туда.
pub async fn fetch(
    url: &str,
    user_agent: &str,
    timeout_secs: u64,
    send_hwid: bool,
    custom_hwid_params: &str,
) -> Result<(String, String)> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;
    let mut request = client.get(url);
    if send_hwid {
        request = request.headers(hwid_headers(custom_hwid_params)?);
    }
    let resp = request
        .send()
        .await
        .with_context(|| format!("запрос к {url}"))?;

    let status = resp.status();
    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !status.is_success() {
        bail!("сервер подписки ответил {status}");
    }
    let body = resp.text().await.context("чтение тела подписки")?;
    Ok((body, userinfo))
}

/// Headers used by subscription servers to identify the device.  Custom values
/// are merged with the platform values, so an omitted custom key keeps the
/// automatic value.
pub fn hwid_headers(custom_params: &str) -> Result<HeaderMap> {
    let mut values = automatic_hwid();
    if !custom_params.is_empty() {
        for (key, value) in parse_custom_hwid_params(custom_params)? {
            values.insert(key, value);
        }
    }

    let mut headers = HeaderMap::new();
    for (key, header_name) in [
        ("hwid", "x-hwid"),
        ("os", "x-device-os"),
        ("osversion", "x-ver-os"),
        ("model", "x-device-model"),
    ] {
        if let Some(value) = values.get(key).filter(|value| !value.is_empty()) {
            headers.insert(
                HeaderName::from_static(header_name),
                HeaderValue::from_str(value)?,
            );
        }
    }
    Ok(headers)
}

fn parse_custom_hwid_params(params: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    for item in params.split(',') {
        let Some((key, value)) = item.split_once('=') else {
            continue;
        };
        let key = key.to_ascii_lowercase();
        if !matches!(key.as_str(), "hwid" | "os" | "osversion" | "model")
            || key.is_empty()
            || value.is_empty()
            || value.contains(['\r', '\n'])
            || value.chars().count() >= 1000
        {
            continue;
        }
        result.insert(key, value.to_string());
    }
    Ok(result)
}

fn automatic_hwid() -> HashMap<String, String> {
    let mut values = HashMap::new();
    values.insert("hwid".into(), machine_id());
    values.insert("os".into(), device_os());
    values.insert("osversion".into(), os_version());
    values.insert("model".into(), device_model());
    values
}

#[cfg(target_os = "linux")]
fn machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(not(target_os = "linux"))]
fn machine_id() -> String {
    String::new()
}

#[cfg(target_os = "linux")]
fn device_os() -> String {
    "Linux".into()
}

#[cfg(not(target_os = "linux"))]
fn device_os() -> String {
    std::env::consts::OS.into()
}

#[cfg(target_os = "linux")]
fn os_version() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(not(target_os = "linux"))]
fn os_version() -> String {
    String::new()
}

#[cfg(target_os = "linux")]
fn device_model() -> String {
    std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(not(target_os = "linux"))]
fn device_model() -> String {
    String::new()
}

/// Разбирает тело подписки в любом из известных форматов.
pub fn parse(body: &str) -> Result<Parsed> {
    let text = body.trim();
    if text.is_empty() {
        bail!("подписка пуста");
    }

    // JSON: либо массив outbound-ов, либо конфиг sing-box целиком.
    if text.starts_with('{') || text.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            return Ok(parse_json(&v));
        }
    }

    // Clash: ищем ключ верхнего уровня, а не просто наличие двоеточия —
    // base64 без паддинга тоже бывает похож на YAML.
    if text.contains("proxies:") {
        if let Ok(parsed) = parse_clash(text) {
            return Ok(parsed);
        }
    }

    // Список ссылок: как есть или в base64.
    if let Some(decoded) = try_decode_body(text) {
        let (profiles, errors) = link::parse_many(&decoded);
        if !profiles.is_empty() {
            return Ok(Parsed {
                profiles,
                errors,
                info: String::new(),
            });
        }
    }

    let (profiles, errors) = link::parse_many(text);
    if profiles.is_empty() {
        bail!(
            "не удалось распознать формат подписки{}",
            if errors.is_empty() {
                String::new()
            } else {
                format!(" ({})", errors.join("; "))
            }
        );
    }
    Ok(Parsed {
        profiles,
        errors,
        info: String::new(),
    })
}

fn try_decode_body(text: &str) -> Option<String> {
    // Тело в base64 не содержит `://` — по этому и отличаем его от списка ссылок.
    if text.contains("://") {
        return None;
    }
    link::decode_b64(text)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

fn parse_json(v: &Value) -> Parsed {
    let outbounds = match v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .get("outbounds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let mut out = Parsed::default();
    for ob in outbounds {
        // Служебные outbound-ы конфига — не серверы.
        let kind = ob.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(
            kind,
            "direct" | "block" | "dns" | "selector" | "urltest" | ""
        ) {
            continue;
        }
        match Profile::from_outbound(ob) {
            Ok(p) => out.profiles.push(p),
            Err(e) => out.errors.push(e.to_string()),
        }
    }
    out
}

/// Clash-конфиг. Переводим каждый `proxies[]` в ссылку и отдаём общему парсеру:
/// поля там те же, что в ссылках, только разложены по ключам.
fn parse_clash(text: &str) -> Result<Parsed> {
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).context("разбор YAML")?;
    let proxies = doc
        .get("proxies")
        .and_then(|p| p.as_sequence())
        .cloned()
        .unwrap_or_default();
    if proxies.is_empty() {
        bail!("в Clash-конфиге нет секции proxies");
    }

    let mut out = Parsed::default();
    for proxy in proxies {
        let name = ys(&proxy, "name");
        match clash_proxy_to_outbound(&proxy) {
            Ok(mut profile) => {
                if !name.is_empty() {
                    profile.name = name;
                }
                out.profiles.push(profile);
            }
            Err(e) => out.errors.push(format!("{name}: {e}")),
        }
    }
    Ok(out)
}

fn ys(v: &serde_yaml_ng::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_yaml_ng::Value::String(s)) => s.clone(),
        Some(serde_yaml_ng::Value::Number(n)) => n.to_string(),
        Some(serde_yaml_ng::Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn yn(v: &serde_yaml_ng::Value, key: &str) -> Option<u64> {
    match v.get(key) {
        Some(serde_yaml_ng::Value::Number(n)) => n.as_u64(),
        Some(serde_yaml_ng::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn yb(v: &serde_yaml_ng::Value, key: &str) -> bool {
    matches!(v.get(key), Some(serde_yaml_ng::Value::Bool(true))) || ys(v, key) == "true"
}

fn clash_proxy_to_outbound(p: &serde_yaml_ng::Value) -> Result<Profile> {
    use serde_json::{json, Map};

    let kind = ys(p, "type");
    let server = ys(p, "server");
    let port = yn(p, "port").unwrap_or(0) as u16;
    if server.is_empty() || port == 0 {
        bail!("нет адреса сервера");
    }

    let mut o = Map::new();
    o.insert("server".into(), json!(server.clone()));
    o.insert("server_port".into(), json!(port));

    let mut tls = Map::new();
    if yb(p, "tls") || kind == "trojan" || kind == "hysteria2" || kind == "tuic" {
        tls.insert("enabled".into(), json!(true));
        let sni = match ys(p, "sni") {
            s if !s.is_empty() => s,
            _ => ys(p, "servername"),
        };
        if !sni.is_empty() {
            tls.insert("server_name".into(), json!(sni));
        }
        if yb(p, "skip-cert-verify") {
            tls.insert("insecure".into(), json!(true));
        }
        if let Some(alpn) = p.get("alpn").and_then(|a| a.as_sequence()) {
            let list: Vec<String> = alpn
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !list.is_empty() {
                tls.insert("alpn".into(), json!(list));
            }
        }
        let fp = ys(p, "client-fingerprint");
        if !fp.is_empty() {
            tls.insert("utls".into(), json!({"enabled": true, "fingerprint": fp}));
        }
        if let Some(reality) = p.get("reality-opts") {
            tls.insert(
                "reality".into(),
                json!({
                    "enabled": true,
                    "public_key": ys(reality, "public-key"),
                    "short_id": ys(reality, "short-id"),
                }),
            );
        }
    }

    let transport = match ys(p, "network").as_str() {
        "ws" => {
            let opts = p.get("ws-opts");
            let mut t = json!({"type": "ws"});
            if let Some(opts) = opts {
                t["path"] = json!(match ys(opts, "path").as_str() {
                    "" => "/".to_string(),
                    s => s.to_string(),
                });
                if let Some(headers) = opts.get("headers") {
                    let host = ys(headers, "Host");
                    if !host.is_empty() {
                        t["headers"] = json!({ "Host": host });
                    }
                }
            }
            Some(t)
        }
        "grpc" => Some(json!({
            "type": "grpc",
            "service_name": p.get("grpc-opts").map(|o| ys(o, "grpc-service-name")).unwrap_or_default(),
        })),
        "h2" => Some(json!({"type": "http"})),
        _ => None,
    };

    match kind.as_str() {
        "vless" => {
            o.insert("type".into(), json!("vless"));
            o.insert("uuid".into(), json!(ys(p, "uuid")));
            let flow = ys(p, "flow");
            if !flow.is_empty() {
                o.insert("flow".into(), json!(flow));
            }
            o.insert("packet_encoding".into(), json!("xudp"));
        }
        "vmess" => {
            o.insert("type".into(), json!("vmess"));
            o.insert("uuid".into(), json!(ys(p, "uuid")));
            o.insert("alter_id".into(), json!(yn(p, "alterId").unwrap_or(0)));
            o.insert(
                "security".into(),
                json!(match ys(p, "cipher").as_str() {
                    "" => "auto".to_string(),
                    s => s.to_string(),
                }),
            );
            o.insert("packet_encoding".into(), json!("xudp"));
        }
        "trojan" => {
            o.insert("type".into(), json!("trojan"));
            o.insert("password".into(), json!(ys(p, "password")));
        }
        "ss" => {
            o.insert("type".into(), json!("shadowsocks"));
            o.insert("method".into(), json!(ys(p, "cipher")));
            o.insert("password".into(), json!(ys(p, "password")));
        }
        "hysteria2" => {
            o.insert("type".into(), json!("hysteria2"));
            o.insert("password".into(), json!(ys(p, "password")));
            if let Some(obfs_pass) = p.get("obfs-password") {
                o.insert(
                    "obfs".into(),
                    json!({
                        "type": match ys(p, "obfs").as_str() { "" => "salamander", s => s },
                        "password": obfs_pass.as_str().unwrap_or_default(),
                    }),
                );
            }
        }
        "tuic" => {
            o.insert("type".into(), json!("tuic"));
            o.insert("uuid".into(), json!(ys(p, "uuid")));
            o.insert("password".into(), json!(ys(p, "password")));
            o.insert(
                "congestion_control".into(),
                json!(match ys(p, "congestion-controller").as_str() {
                    "" => "bbr",
                    s => s,
                }),
            );
        }
        "socks5" => {
            o.insert("type".into(), json!("socks"));
            o.insert("version".into(), json!("5"));
            let user = ys(p, "username");
            if !user.is_empty() {
                o.insert("username".into(), json!(user));
                o.insert("password".into(), json!(ys(p, "password")));
            }
        }
        "http" => {
            o.insert("type".into(), json!("http"));
            let user = ys(p, "username");
            if !user.is_empty() {
                o.insert("username".into(), json!(user));
                o.insert("password".into(), json!(ys(p, "password")));
            }
        }
        other => bail!("тип `{other}` не поддерживается"),
    }

    if !tls.is_empty() {
        o.insert("tls".into(), Value::Object(tls));
    }
    if let Some(t) = transport {
        o.insert("transport".into(), t);
    }
    Profile::from_outbound(Value::Object(o))
}

/// Разбирает заголовок `subscription-userinfo` в человекочитаемую строку.
/// Формат: `upload=0; download=1234; total=5678; expire=1700000000`.
pub fn format_userinfo(header: &str) -> String {
    let mut used = 0u64;
    let mut total = 0u64;
    let mut expire = 0i64;
    for part in header.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else {
            continue;
        };
        match k.trim() {
            "upload" | "download" => used += v.trim().parse().unwrap_or(0),
            "total" => total = v.trim().parse().unwrap_or(0),
            "expire" => expire = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if total > 0 {
        parts.push(format!("{} из {}", human_bytes(used), human_bytes(total)));
    } else if used > 0 {
        parts.push(human_bytes(used));
    }
    if expire > 0 {
        parts.push(format!("до {}", format_date(expire)));
    }
    parts.join(", ")
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["Б", "КиБ", "МиБ", "ГиБ", "ТиБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Дата в ISO без внешних зависимостей: алгоритм Хиннанта (civil_from_days).
fn format_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    #[test]
    fn base64_list_is_decoded() {
        let list = "trojan://pw@a.example:443#a\nvless://u@b.example:443#b";
        let parsed = parse(&STANDARD.encode(list)).unwrap();
        assert_eq!(parsed.profiles.len(), 2);
        assert_eq!(parsed.profiles[0].name, "a");
    }

    #[test]
    fn plain_list_is_accepted() {
        let parsed = parse("trojan://pw@a.example:443#a\n").unwrap();
        assert_eq!(parsed.profiles.len(), 1);
    }

    #[test]
    fn singbox_json_skips_service_outbounds() {
        let body = r#"{"outbounds":[
            {"type":"direct","tag":"direct"},
            {"type":"selector","tag":"proxy","outbounds":["a"]},
            {"type":"trojan","tag":"a","server":"a.example","server_port":443,"password":"pw"}
        ]}"#;
        let parsed = parse(body).unwrap();
        assert_eq!(parsed.profiles.len(), 1);
        assert_eq!(parsed.profiles[0].name, "a");
    }

    #[test]
    fn clash_yaml_is_translated() {
        let body = r#"
proxies:
  - name: "de-1"
    type: vless
    server: de.example
    port: 443
    uuid: UUID
    tls: true
    servername: de.example
    network: ws
    client-fingerprint: chrome
    ws-opts:
      path: /ws
      headers:
        Host: cdn.example
"#;
        let parsed = parse(body).unwrap();
        assert_eq!(parsed.profiles.len(), 1);
        let p = &parsed.profiles[0];
        assert_eq!(p.name, "de-1");
        assert_eq!(p.outbound["uuid"], "UUID");
        assert_eq!(p.outbound["tls"]["server_name"], "de.example");
        assert_eq!(p.outbound["transport"]["headers"]["Host"], "cdn.example");
    }

    #[test]
    fn empty_body_is_an_error() {
        assert!(parse("   ").is_err());
    }

    #[test]
    fn userinfo_header_is_humanised() {
        let s = format_userinfo(
            "upload=1073741824; download=1073741824; total=107374182400; expire=1801607400",
        );
        assert!(s.starts_with("2.0 ГиБ из 100.0 ГиБ"), "{s}");
        assert!(s.contains("до 2027-"), "{s}");
    }

    #[test]
    fn custom_hwid_params_override_automatic_values() {
        let headers = hwid_headers("HWID=custom,os=Android,osVersion=13,model=Phone").unwrap();
        assert_eq!(headers["x-hwid"], "custom");
        assert_eq!(headers["x-device-os"], "Android");
        assert_eq!(headers["x-ver-os"], "13");
        assert_eq!(headers["x-device-model"], "Phone");
    }

    #[test]
    fn custom_hwid_params_ignore_invalid_values() {
        for params in ["unknown=value", "hwid=", "hwid=a\n", "hwid=a,bad"] {
            assert!(hwid_headers(params).is_ok(), "rejected {params:?}");
        }
        assert!(hwid_headers(&format!("hwid={}", "x".repeat(1000))).is_ok());
    }

    #[test]
    fn bytes_are_formatted() {
        assert_eq!(human_bytes(512), "512 Б");
        assert_eq!(human_bytes(1536), "1.5 КиБ");
    }
}
