//! Parsing server links into sing-box outbounds.
//!
//! Links in this ecosystem are folklore, not a standard: the same protocol
//! is encoded differently by different panels. Therefore the parser is deliberately lenient:
//! it fixes base64 padding, accepts both `hy2://` and `hysteria2://`, and handles
//! unescaped `#` in names, but never guesses anything that affects
//! the connection: a missing host, port, or key is an error.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::{json, Map, Value};

use crate::profile::Profile;

/// Base64 in links appears in all four combinations of alphabet and
/// padding; try them in turn before declaring the string invalid.
pub fn decode_b64(s: &str) -> Result<Vec<u8>> {
    let t: String = s.trim().chars().filter(|c| !c.is_whitespace()).collect();
    for engine in [
        &STANDARD as &dyn Fn2,
        &STANDARD_NO_PAD as &dyn Fn2,
        &URL_SAFE_NO_PAD as &dyn Fn2,
    ] {
        if let Ok(v) = engine.decode2(&t) {
            return Ok(v);
        }
    }
    bail!("строка не является корректным base64")
}

/// A small trait instead of enumerating base64 engine types: they have different types,
/// but one call is needed.
trait Fn2 {
    fn decode2(&self, s: &str) -> Result<Vec<u8>>;
}
impl<T: Engine> Fn2 for T {
    fn decode2(&self, s: &str) -> Result<Vec<u8>> {
        Ok(self.decode(s)?)
    }
}

fn urldecode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// A link split into parts. This is custom rather than `url::Url`: profile names regularly
/// contain raw `#`, `%`, and spaces that break a strict parser,
/// and panels produce such links every day.
struct Raw<'a> {
    scheme: &'a str,
    /// Everything between `://` and `?`/`#`.
    body: String,
    query: Vec<(String, String)>,
    fragment: String,
}

impl<'a> Raw<'a> {
    fn parse(link: &'a str) -> Result<Self> {
        let link = link.trim();
        let (scheme, rest) = link
            .split_once("://")
            .ok_or_else(|| anyhow!("в ссылке нет схемы `протокол://`"))?;

        // Cut the fragment at the first `#`: the name may contain anything.
        let (before_frag, fragment) = match rest.split_once('#') {
            Some((b, f)) => (b, urldecode(f)),
            None => (rest, String::new()),
        };
        let (body, query_str) = match before_frag.split_once('?') {
            Some((b, q)) => (b, q),
            None => (before_frag, ""),
        };

        let query = query_str
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) => (urldecode(k), urldecode(v)),
                None => (urldecode(pair), String::new()),
            })
            .collect();

        Ok(Self {
            scheme,
            body: body.to_string(),
            query,
            fragment,
        })
    }

    fn q(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    fn q_or<'b>(&'b self, key: &str, default: &'b str) -> &'b str {
        self.q(key).unwrap_or(default)
    }

    fn q_bool(&self, key: &str) -> bool {
        matches!(self.q(key), Some("1" | "true" | "True"))
    }

    /// `user@host:port` → (user, host, port). user may be empty.
    fn userinfo_host_port(&self) -> Result<(String, String, u16)> {
        let (user, hostport) = match self.body.rsplit_once('@') {
            Some((u, h)) => (urldecode(u), h),
            None => (String::new(), self.body.as_str()),
        };
        let (host, port) = split_host_port(hostport)?;
        Ok((user, host, port))
    }
}

fn split_host_port(s: &str) -> Result<(String, u16)> {
    // IPv6 in a link is always bracketed: [::1]:443
    if let Some(rest) = s.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("незакрытая скобка в IPv6-адресе"))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| anyhow!("после IPv6-адреса нет порта"))?;
        return Ok((host.to_string(), parse_port(port)?));
    }
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("в адресе `{s}` нет порта"))?;
    if host.is_empty() {
        bail!("в адресе `{s}` нет хоста");
    }
    Ok((host.to_string(), parse_port(port)?))
}

fn parse_port(s: &str) -> Result<u16> {
    s.parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| anyhow!("`{s}` не похоже на номер порта"))
}

/// Parses one link. The profile name is taken from the fragment, or, if absent,
/// from the address so nameless entries do not appear in the list.
pub fn parse(link: &str) -> Result<Profile> {
    let link = link.trim();
    let raw = Raw::parse(link)?;
    let mut profile = match raw.scheme {
        "vless" => parse_vless(&raw)?,
        "vmess" => parse_vmess(link)?,
        "trojan" => parse_trojan(&raw)?,
        "ss" => parse_shadowsocks(&raw)?,
        "hysteria2" | "hy2" => parse_hysteria2(&raw)?,
        "tuic" => parse_tuic(&raw)?,
        "socks" | "socks5" => parse_socks(&raw)?,
        "http" | "https" => parse_http(&raw, raw.scheme == "https")?,
        "anytls" => parse_anytls(&raw)?,
        "naive+https" | "naive+quic" => parse_naive(&raw)?,
        other => bail!("протокол `{other}` не поддерживается"),
    };

    if !raw.fragment.is_empty() {
        profile.name = raw.fragment.clone();
    }
    if profile.name.is_empty() {
        profile.name = profile.address();
    }
    Ok(profile)
}

/// Parses a list of links separated by newlines. Returns what could be parsed
/// and errors for the rest: one malformed line in a subscription of
/// two hundred servers should not abort the entire import.
pub fn parse_many(text: &str) -> (Vec<Profile>, Vec<String>) {
    let mut ok = Vec::new();
    let mut errors = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        match parse(line) {
            Ok(p) => ok.push(p),
            Err(e) => errors.push(format!("{}: {e}", truncate(line, 60))),
        }
    }
    (ok, errors)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

// ── shared outbound components ───────────────────────────────────────────

/// Builds the `tls` section from v2ray-style query parameters.
/// Returns `None` when encryption is absent.
fn tls_from_query(raw: &Raw, default_sni: &str) -> Option<Value> {
    let security = raw.q_or("security", "none");
    let has_reality = raw.q("pbk").is_some();
    if security == "none" && !has_reality {
        return None;
    }
    Some(build_tls(raw, default_sni, has_reality))
}

/// The same, but for protocols where encryption cannot be disabled (trojan, QUIC-based,
/// anytls): `security` is usually omitted from the link, while `insecure`, `alpn`, and
/// the fingerprint are included and must not be lost.
fn tls_always(raw: &Raw, default_sni: &str) -> Value {
    let mut tls = build_tls(raw, default_sni, raw.q("pbk").is_some());
    tls["enabled"] = json!(true);
    if tls
        .get("server_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
        && !default_sni.is_empty()
    {
        tls["server_name"] = json!(default_sni);
    }
    tls
}

fn build_tls(raw: &Raw, default_sni: &str, has_reality: bool) -> Value {
    let mut tls = Map::new();
    tls.insert("enabled".into(), json!(true));

    let sni = raw
        .q("sni")
        .or_else(|| raw.q("peer"))
        .or_else(|| raw.q("host"))
        .unwrap_or(default_sni);
    if !sni.is_empty() {
        tls.insert("server_name".into(), json!(sni));
    }
    if raw.q_bool("allowInsecure") || raw.q_bool("insecure") || raw.q_bool("allow_insecure") {
        tls.insert("insecure".into(), json!(true));
    }
    if let Some(alpn) = raw.q("alpn") {
        let list: Vec<&str> = alpn
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            tls.insert("alpn".into(), json!(list));
        }
    }
    // TLS fingerprint: reality requires it, so use chrome by default.
    let fp = raw
        .q("fp")
        .unwrap_or(if has_reality { "chrome" } else { "" });
    if !fp.is_empty() {
        tls.insert("utls".into(), json!({"enabled": true, "fingerprint": fp}));
    }
    if let Some(pbk) = raw.q("pbk") {
        tls.insert(
            "reality".into(),
            json!({
                "enabled": true,
                "public_key": pbk,
                "short_id": raw.q_or("sid", ""),
            }),
        );
    }
    Value::Object(tls)
}

/// The `transport` section. `None` means ordinary TCP without a wrapper.
fn transport_from_query(raw: &Raw) -> Option<Value> {
    let kind = raw.q_or("type", "tcp");
    let host = raw.q_or("host", "");
    let path = raw.q_or("path", "/");

    match kind {
        "ws" => {
            let mut t = json!({"type": "ws"});
            // Panels put early data in the path: /path?ed=2048
            let (p, ed) = match path.split_once("?ed=") {
                Some((p, ed)) => (p, ed.parse::<u32>().ok()),
                None => (path, None),
            };
            t["path"] = json!(p);
            if !host.is_empty() {
                t["headers"] = json!({ "Host": host });
            }
            if let Some(ed) = ed.or_else(|| raw.q("ed").and_then(|v| v.parse().ok())) {
                t["max_early_data"] = json!(ed);
                t["early_data_header_name"] = json!("Sec-WebSocket-Protocol");
            }
            Some(t)
        }
        "grpc" => Some(json!({
            "type": "grpc",
            "service_name": raw.q("serviceName").or_else(|| raw.q("path")).unwrap_or(""),
        })),
        "http" | "h2" => {
            let mut t = json!({"type": "http", "path": path});
            if !host.is_empty() {
                t["host"] = json!(host.split(',').map(str::trim).collect::<Vec<_>>());
            }
            Some(t)
        }
        "httpupgrade" => {
            let mut t = json!({"type": "httpupgrade", "path": path});
            if !host.is_empty() {
                t["host"] = json!(host);
            }
            Some(t)
        }
        // xhttp is supported only by Xray; the profile protocol tag is switched above.
        "xhttp" | "splithttp" => {
            let mut t = json!({"type": "xhttp", "path": path, "mode": raw.q_or("mode", "auto")});
            if !host.is_empty() {
                t["host"] = json!(host);
            }
            if let Some(extra) = raw.q("extra") {
                if let Ok(v) = serde_json::from_str::<Value>(extra) {
                    t["extra"] = v;
                }
            }
            Some(t)
        }
        _ => None,
    }
}

fn insert_opt(obj: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        obj.insert(key.into(), v);
    }
}

// ── protocols ────────────────────────────────────────────────────────────

fn parse_vless(raw: &Raw) -> Result<Profile> {
    let (uuid, host, port) = raw.userinfo_host_port()?;
    if uuid.is_empty() {
        bail!("в vless-ссылке нет UUID");
    }
    let mut o = Map::new();
    o.insert("type".into(), json!("vless"));
    o.insert("server".into(), json!(host));
    o.insert("server_port".into(), json!(port));
    o.insert("uuid".into(), json!(uuid));

    let flow = raw.q_or("flow", "");
    if !flow.is_empty() {
        o.insert("flow".into(), json!(flow));
    }
    // xudp provides working UDP where it would otherwise silently fail.
    o.insert("packet_encoding".into(), json!("xudp"));
    insert_opt(&mut o, "tls", tls_from_query(raw, raw.q_or("sni", "")));
    let transport = transport_from_query(raw);
    let is_xhttp = matches!(raw.q_or("type", "tcp"), "xhttp" | "splithttp");
    insert_opt(&mut o, "transport", transport);

    let mut p = Profile::from_outbound(Value::Object(o))?;
    if is_xhttp {
        p.kind = "xrayvless".into();
    }
    Ok(p)
}

/// vmess://<base64 of v2rayN JSON>. Numbers in this JSON may also be strings,
/// hence `as_u16`/`as_u32` instead of reading fields directly.
fn parse_vmess(link: &str) -> Result<Profile> {
    let payload = link.trim_start_matches("vmess://");
    let decoded = decode_b64(payload).context("тело vmess-ссылки")?;
    let v: Value = serde_json::from_slice(&decoded).context("JSON внутри vmess-ссылки")?;

    let get = |k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let as_str = |k: &str| match get(k) {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let as_num = |k: &str| -> Option<u64> {
        match get(k) {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    };

    let host = as_str("add");
    let port = as_num("port")
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| anyhow!("в vmess-ссылке нет корректного порта"))?;
    let uuid = as_str("id");
    if host.is_empty() || uuid.is_empty() {
        bail!("в vmess-ссылке нет адреса или UUID");
    }

    let mut o = Map::new();
    o.insert("type".into(), json!("vmess"));
    o.insert("server".into(), json!(host));
    o.insert("server_port".into(), json!(port));
    o.insert("uuid".into(), json!(uuid));
    o.insert("alter_id".into(), json!(as_num("aid").unwrap_or(0)));
    let security = match as_str("scy").as_str() {
        "" => "auto".to_string(),
        s => s.to_string(),
    };
    o.insert("security".into(), json!(security));
    o.insert("packet_encoding".into(), json!("xudp"));

    // Transport and TLS in vmess JSON use their own keys; convert them
    // to the same query-parameter form understood by the shared code.
    let net = as_str("net");
    let tls_mode = as_str("tls");
    let sni = match as_str("sni") {
        s if !s.is_empty() => s,
        _ => as_str("host"),
    };
    let query = vec![
        ("type".to_string(), net.clone()),
        ("host".to_string(), as_str("host")),
        ("path".to_string(), as_str("path")),
        ("serviceName".to_string(), as_str("path")),
        ("security".to_string(), tls_mode.clone()),
        ("sni".to_string(), sni),
        ("alpn".to_string(), as_str("alpn")),
        ("fp".to_string(), as_str("fp")),
    ];
    let synth = Raw {
        scheme: "vmess",
        body: String::new(),
        query,
        fragment: String::new(),
    };
    if tls_mode == "tls" || tls_mode == "reality" {
        insert_opt(&mut o, "tls", tls_from_query(&synth, ""));
    }
    insert_opt(&mut o, "transport", transport_from_query(&synth));

    let mut p = Profile::from_outbound(Value::Object(o))?;
    p.name = as_str("ps");
    Ok(p)
}

fn parse_trojan(raw: &Raw) -> Result<Profile> {
    let (password, host, port) = raw.userinfo_host_port()?;
    if password.is_empty() {
        bail!("в trojan-ссылке нет пароля");
    }
    let mut o = Map::new();
    o.insert("type".into(), json!("trojan"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    o.insert("password".into(), json!(password));
    // Trojan encryption is always enabled, even when `security` is omitted.
    o.insert("tls".into(), tls_always(raw, &host));
    insert_opt(&mut o, "transport", transport_from_query(raw));
    Profile::from_outbound(Value::Object(o))
}

/// Two incompatible formats under one scheme: SIP002 (`ss://base64(method:pass)@host:port`)
/// and the old fully base64-encoded format (`ss://base64(method:pass@host:port)`).
fn parse_shadowsocks(raw: &Raw) -> Result<Profile> {
    let (method, password, host, port) = match raw.body.rsplit_once('@') {
        Some((userinfo, hostport)) => {
            let (host, port) = split_host_port(hostport)?;
            let decoded = decode_b64(userinfo)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| urldecode(userinfo));
            let (method, password) = decoded
                .split_once(':')
                .ok_or_else(|| anyhow!("в ss-ссылке не разобрать метод и пароль"))?;
            (method.to_string(), password.to_string(), host, port)
        }
        None => {
            let decoded = String::from_utf8(decode_b64(&raw.body).context("тело ss-ссылки")?)
                .context("тело ss-ссылки не UTF-8")?;
            let (userinfo, hostport) = decoded
                .rsplit_once('@')
                .ok_or_else(|| anyhow!("в ss-ссылке нет адреса сервера"))?;
            let (host, port) = split_host_port(hostport)?;
            let (method, password) = userinfo
                .split_once(':')
                .ok_or_else(|| anyhow!("в ss-ссылке не разобрать метод и пароль"))?;
            (method.to_string(), password.to_string(), host, port)
        }
    };

    let mut o = Map::new();
    o.insert("type".into(), json!("shadowsocks"));
    o.insert("server".into(), json!(host));
    o.insert("server_port".into(), json!(port));
    o.insert("method".into(), json!(method));
    o.insert("password".into(), json!(password));
    if let Some(plugin) = raw.q("plugin") {
        // plugin=obfs-local;obfs=tls;obfs-host=example.com
        let (name, opts) = plugin.split_once(';').unwrap_or((plugin, ""));
        o.insert("plugin".into(), json!(name));
        if !opts.is_empty() {
            o.insert("plugin_opts".into(), json!(opts));
        }
    }
    Profile::from_outbound(Value::Object(o))
}

fn parse_hysteria2(raw: &Raw) -> Result<Profile> {
    let (password, host, port) = raw.userinfo_host_port()?;
    let mut o = Map::new();
    o.insert("type".into(), json!("hysteria2"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    o.insert("password".into(), json!(password));

    if let Some(obfs_pass) = raw.q("obfs-password") {
        o.insert(
            "obfs".into(),
            json!({"type": raw.q_or("obfs", "salamander"), "password": obfs_pass}),
        );
    }
    for (key, field) in [("upmbps", "up_mbps"), ("downmbps", "down_mbps")] {
        if let Some(v) = raw.q(key).and_then(|v| v.parse::<u32>().ok()) {
            o.insert(field.into(), json!(v));
        }
    }
    // QUIC cannot work without TLS; enable it even if the link omits it.
    o.insert("tls".into(), tls_always(raw, &host));
    Profile::from_outbound(Value::Object(o))
}

fn parse_tuic(raw: &Raw) -> Result<Profile> {
    let (userinfo, host, port) = raw.userinfo_host_port()?;
    let (uuid, password) = userinfo.split_once(':').unwrap_or((userinfo.as_str(), ""));
    if uuid.is_empty() {
        bail!("в tuic-ссылке нет UUID");
    }
    let mut o = Map::new();
    o.insert("type".into(), json!("tuic"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    o.insert("uuid".into(), json!(uuid));
    o.insert("password".into(), json!(password));
    o.insert(
        "congestion_control".into(),
        json!(raw.q_or("congestion_control", "bbr")),
    );
    o.insert(
        "udp_relay_mode".into(),
        json!(raw.q_or("udp_relay_mode", "native")),
    );
    o.insert("tls".into(), tls_always(raw, &host));
    Profile::from_outbound(Value::Object(o))
}

fn parse_socks(raw: &Raw) -> Result<Profile> {
    let (userinfo, host, port) = raw.userinfo_host_port()?;
    // socks://base64(user:pass)@host:port also occurs.
    let userinfo = match decode_b64(&userinfo) {
        Ok(bytes) if !userinfo.contains(':') && !userinfo.is_empty() => {
            String::from_utf8(bytes).unwrap_or(userinfo)
        }
        _ => userinfo,
    };
    let mut o = Map::new();
    o.insert("type".into(), json!("socks"));
    o.insert("server".into(), json!(host));
    o.insert("server_port".into(), json!(port));
    o.insert("version".into(), json!("5"));
    if let Some((user, pass)) = userinfo.split_once(':') {
        if !user.is_empty() {
            o.insert("username".into(), json!(user));
            o.insert("password".into(), json!(pass));
        }
    }
    Profile::from_outbound(Value::Object(o))
}

fn parse_http(raw: &Raw, tls: bool) -> Result<Profile> {
    let (userinfo, host, port) = raw.userinfo_host_port()?;
    let mut o = Map::new();
    o.insert("type".into(), json!("http"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    if let Some((user, pass)) = userinfo.split_once(':') {
        if !user.is_empty() {
            o.insert("username".into(), json!(user));
            o.insert("password".into(), json!(pass));
        }
    }
    if tls {
        o.insert(
            "tls".into(),
            json!({"enabled": true, "server_name": raw.q_or("sni", &host)}),
        );
    }
    Profile::from_outbound(Value::Object(o))
}

/// naive+https:// and naive+quic://. Encryption is always enabled; the port defaults to
/// 443 because panels usually omit it.
fn parse_naive(raw: &Raw) -> Result<Profile> {
    let (userinfo, host, port) = match raw.userinfo_host_port() {
        Ok(parts) => parts,
        Err(_) => {
            let (user, hostport) = match raw.body.rsplit_once('@') {
                Some((u, h)) => (urldecode(u), h.to_string()),
                None => (String::new(), raw.body.clone()),
            };
            if hostport.is_empty() {
                bail!("в naive-ссылке нет адреса сервера");
            }
            (user, hostport, 443)
        }
    };

    let mut o = Map::new();
    o.insert("type".into(), json!("naive"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    if let Some((user, password)) = userinfo.split_once(':') {
        if !user.is_empty() {
            o.insert("username".into(), json!(user));
            o.insert("password".into(), json!(password));
        }
    }
    if raw.scheme == "naive+quic" {
        o.insert("quic".into(), json!(true));
        if let Some(cc) = raw.q("congestion_control") {
            o.insert("quic_congestion_control".into(), json!(cc));
        }
    }
    if raw.q_bool("uot") {
        o.insert("udp_over_tcp".into(), json!(true));
    }
    o.insert("tls".into(), tls_always(raw, &host));
    Profile::from_outbound(Value::Object(o))
}

fn parse_anytls(raw: &Raw) -> Result<Profile> {
    let (password, host, port) = raw.userinfo_host_port()?;
    let mut o = Map::new();
    o.insert("type".into(), json!("anytls"));
    o.insert("server".into(), json!(host.clone()));
    o.insert("server_port".into(), json!(port));
    o.insert("password".into(), json!(password));
    o.insert("tls".into(), tls_always(raw, &host));
    Profile::from_outbound(Value::Object(o))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vless_reality_with_vision() {
        let p = parse(
            "vless://edfbda8a-7db5-4885-84e8-0a09e15c79a5@de1.example.com:443\
             ?security=reality&sni=de1.example.com&fp=firefox&pbk=PUBKEY&sid=ba15d0ae\
             &flow=xtls-rprx-vision&type=tcp#%F0%9F%87%A9%F0%9F%87%AA%20Germany",
        )
        .unwrap();
        assert_eq!(p.kind, "vless");
        assert_eq!(p.name, "🇩🇪 Germany");
        assert_eq!(p.outbound["flow"], "xtls-rprx-vision");
        assert_eq!(p.outbound["tls"]["reality"]["public_key"], "PUBKEY");
        assert_eq!(p.outbound["tls"]["utls"]["fingerprint"], "firefox");
        assert!(p.outbound.get("transport").is_none());
    }

    #[test]
    fn vless_ws_early_data_in_path() {
        let p = parse("vless://uuid@a.example:443?type=ws&path=%2Fws%3Fed%3D2048&host=cdn.example&security=tls#ws")
            .unwrap();
        let t = &p.outbound["transport"];
        assert_eq!(t["type"], "ws");
        assert_eq!(t["path"], "/ws");
        assert_eq!(t["max_early_data"], 2048);
        assert_eq!(t["headers"]["Host"], "cdn.example");
    }

    #[test]
    fn vless_xhttp_is_marked_for_xray() {
        let p =
            parse("vless://uuid@a.example:443?type=xhttp&security=tls&path=%2Fx&mode=packet-up#x")
                .unwrap();
        assert_eq!(p.kind, "xrayvless");
        assert_eq!(p.outbound["transport"]["type"], "xhttp");
        assert_eq!(p.outbound["transport"]["mode"], "packet-up");
    }

    #[test]
    fn vmess_with_string_numbers() {
        let payload = STANDARD.encode(
            r#"{"v":"2","ps":"tokyo","add":"jp.example","port":"443","id":"UUID","aid":"0",
                "net":"ws","path":"/p","host":"cdn.example","tls":"tls","sni":"cdn.example"}"#,
        );
        let p = parse(&format!("vmess://{payload}")).unwrap();
        assert_eq!(p.name, "tokyo");
        assert_eq!(p.outbound["server_port"], 443);
        assert_eq!(p.outbound["alter_id"], 0);
        assert_eq!(p.outbound["transport"]["path"], "/p");
        assert_eq!(p.outbound["tls"]["server_name"], "cdn.example");
    }

    #[test]
    fn shadowsocks_sip002_and_legacy_agree() {
        let sip002 = format!(
            "ss://{}@ss.example:8388#name",
            URL_SAFE_NO_PAD.encode("aes-256-gcm:secret")
        );
        let legacy = format!(
            "ss://{}#name",
            STANDARD.encode("aes-256-gcm:secret@ss.example:8388")
        );
        let a = parse(&sip002).unwrap();
        let b = parse(&legacy).unwrap();
        assert_eq!(a.outbound, b.outbound);
        assert_eq!(a.outbound["method"], "aes-256-gcm");
        assert_eq!(a.outbound["password"], "secret");
        assert_eq!(a.outbound["server_port"], 8388);
    }

    #[test]
    fn hysteria2_obfs_and_tls_always_on() {
        let p = parse("hy2://pass@h2.example:443?obfs=salamander&obfs-password=zzz&insecure=1#h2")
            .unwrap();
        assert_eq!(p.kind, "hysteria2");
        assert_eq!(p.outbound["obfs"]["password"], "zzz");
        assert_eq!(p.outbound["tls"]["enabled"], true);
        assert_eq!(p.outbound["tls"]["insecure"], true);
        assert_eq!(p.outbound["tls"]["server_name"], "h2.example");
    }

    #[test]
    fn tuic_splits_uuid_and_password() {
        let p =
            parse("tuic://uuid-here:pass-here@t.example:443?congestion_control=cubic#t").unwrap();
        assert_eq!(p.outbound["uuid"], "uuid-here");
        assert_eq!(p.outbound["password"], "pass-here");
        assert_eq!(p.outbound["congestion_control"], "cubic");
    }

    #[test]
    fn naive_https_and_quic() {
        let p = parse("naive+https://user:pass@n.example:443?uot=1#naive").unwrap();
        assert_eq!(p.kind, "naive");
        assert_eq!(p.outbound["username"], "user");
        assert_eq!(p.outbound["password"], "pass");
        assert_eq!(p.outbound["udp_over_tcp"], true);
        assert_eq!(p.outbound["tls"]["enabled"], true);
        assert!(p.outbound.get("quic").is_none());

        let q = parse("naive+quic://user:pass@n.example:443?congestion_control=bbr#q").unwrap();
        assert_eq!(q.outbound["quic"], true);
        assert_eq!(q.outbound["quic_congestion_control"], "bbr");
    }

    #[test]
    fn naive_without_port_defaults_to_443() {
        let p = parse("naive+https://user:pass@n.example#naive").unwrap();
        assert_eq!(p.outbound["server_port"], 443);
        assert_eq!(p.outbound["server"], "n.example");
    }

    #[test]
    fn ipv6_host_is_unbracketed_in_config() {
        let p = parse("trojan://pw@[2001:db8::1]:443#v6").unwrap();
        assert_eq!(p.outbound["server"], "2001:db8::1");
        assert_eq!(p.address(), "[2001:db8::1]:443");
    }

    #[test]
    fn name_falls_back_to_address() {
        let p = parse("trojan://pw@t.example:443").unwrap();
        assert_eq!(p.name, "t.example:443");
    }

    #[test]
    fn parse_many_keeps_good_lines() {
        let (ok, errs) = parse_many(
            "trojan://pw@a.example:443#a\nне ссылка\nvless://u@b.example:443#b\n# комментарий\n",
        );
        assert_eq!(ok.len(), 2);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn missing_port_is_an_error() {
        assert!(parse("trojan://pw@host-without-port").is_err());
    }
}
