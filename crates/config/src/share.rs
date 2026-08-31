//! The reverse operation of [`crate::link`]: outbound → link.
//!
//! Used for “copy link” and group export. A round trip through
//! `link::parse` must produce the same outbound; tests verify this.

use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde_json::{json, Value};

use crate::profile::Profile;

/// Escape everything that can break link parsing, including `&`, `=`, `#`, and `+`
/// (the latter is read as a space in a query).
const ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn esc(s: &str) -> String {
    utf8_percent_encode(s, ESCAPE).to_string()
}

fn host_for_url(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// A profile link in a format understood by other clients.
pub fn to_link(profile: &Profile) -> Result<String> {
    let o = &profile.outbound;
    let kind = o["type"].as_str().unwrap_or_default();
    let host = o["server"].as_str().unwrap_or_default();
    let port = o["server_port"].as_i64().unwrap_or(0);
    let name = esc(&profile.name);
    let addr = format!("{}:{port}", host_for_url(host));

    let link = match kind {
        "vless" => {
            let mut q = stream_query(o);
            if let Some(flow) = o["flow"].as_str().filter(|f| !f.is_empty()) {
                q.push(("flow".into(), flow.into()));
            }
            format!(
                "vless://{}@{addr}?{}#{name}",
                esc(o["uuid"].as_str().unwrap_or_default()),
                join(&q)
            )
        }
        "vmess" => {
            // v2rayN format: base64 of JSON, not a query string.
            let transport = &o["transport"];
            let tls = &o["tls"];
            let payload = json!({
                "v": "2",
                "ps": profile.name,
                "add": host,
                "port": port,
                "id": o["uuid"].as_str().unwrap_or_default(),
                "aid": o["alter_id"].as_i64().unwrap_or(0),
                "scy": o["security"].as_str().unwrap_or("auto"),
                "net": match transport["type"].as_str() {
                    Some("http") => "h2",
                    Some(other) => other,
                    None => "tcp",
                },
                "type": "none",
                "host": transport["headers"]["Host"].as_str()
                    .or_else(|| transport["host"].as_str())
                    .unwrap_or_default(),
                "path": transport["path"].as_str()
                    .or_else(|| transport["service_name"].as_str())
                    .unwrap_or_default(),
                "tls": if tls["enabled"] == json!(true) { "tls" } else { "" },
                "sni": tls["server_name"].as_str().unwrap_or_default(),
                "fp": tls["utls"]["fingerprint"].as_str().unwrap_or_default(),
            });
            format!("vmess://{}", STANDARD.encode(payload.to_string()))
        }
        "trojan" => format!(
            "trojan://{}@{addr}?{}#{name}",
            esc(o["password"].as_str().unwrap_or_default()),
            join(&stream_query(o))
        ),
        "shadowsocks" => {
            let userinfo = format!(
                "{}:{}",
                o["method"].as_str().unwrap_or_default(),
                o["password"].as_str().unwrap_or_default()
            );
            format!(
                "ss://{}@{addr}#{name}",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(userinfo)
            )
        }
        "hysteria2" => {
            let mut q = tls_query(o);
            if let Some(obfs) = o["obfs"]["password"].as_str() {
                q.push((
                    "obfs".into(),
                    o["obfs"]["type"].as_str().unwrap_or("salamander").into(),
                ));
                q.push(("obfs-password".into(), obfs.into()));
            }
            format!(
                "hysteria2://{}@{addr}?{}#{name}",
                esc(o["password"].as_str().unwrap_or_default()),
                join(&q)
            )
        }
        "tuic" => {
            let mut q = tls_query(o);
            for (key, field) in [
                ("congestion_control", "congestion_control"),
                ("udp_relay_mode", "udp_relay_mode"),
            ] {
                if let Some(v) = o[field].as_str().filter(|v| !v.is_empty()) {
                    q.push((key.into(), v.into()));
                }
            }
            format!(
                "tuic://{}:{}@{addr}?{}#{name}",
                esc(o["uuid"].as_str().unwrap_or_default()),
                esc(o["password"].as_str().unwrap_or_default()),
                join(&q)
            )
        }
        "anytls" => format!(
            "anytls://{}@{addr}?{}#{name}",
            esc(o["password"].as_str().unwrap_or_default()),
            join(&tls_query(o))
        ),
        "naive" => {
            let scheme = if o["quic"] == json!(true) {
                "naive+quic"
            } else {
                "naive+https"
            };
            let auth = match (o["username"].as_str(), o["password"].as_str()) {
                (Some(u), Some(p)) if !u.is_empty() => format!("{}:{}@", esc(u), esc(p)),
                _ => String::new(),
            };
            let mut q = tls_query(o);
            if o["udp_over_tcp"] == json!(true) {
                q.push(("uot".into(), "1".into()));
            }
            if let Some(cc) = o["quic_congestion_control"].as_str() {
                q.push(("congestion_control".into(), cc.into()));
            }
            format!("{scheme}://{auth}{addr}?{}#{name}", join(&q))
        }
        "socks" => {
            let auth = match (o["username"].as_str(), o["password"].as_str()) {
                (Some(u), Some(p)) if !u.is_empty() => format!("{}:{}@", esc(u), esc(p)),
                _ => String::new(),
            };
            format!("socks5://{auth}{addr}#{name}")
        }
        "http" => {
            let auth = match (o["username"].as_str(), o["password"].as_str()) {
                (Some(u), Some(p)) if !u.is_empty() => format!("{}:{}@", esc(u), esc(p)),
                _ => String::new(),
            };
            let scheme = if o["tls"]["enabled"] == json!(true) {
                "https"
            } else {
                "http"
            };
            format!("{scheme}://{auth}{addr}#{name}")
        }
        other => bail!("для протокола `{other}` ссылка не собирается"),
    };
    Ok(link)
}

/// Links for a set of profiles, one per line.
pub fn to_links(profiles: &[Profile]) -> String {
    profiles
        .iter()
        .filter_map(|p| to_link(p).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

fn join(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", esc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn tls_query(o: &Value) -> Vec<(String, String)> {
    let tls = &o["tls"];
    let mut q = Vec::new();
    if tls["enabled"] != json!(true) {
        q.push(("security".into(), "none".into()));
        return q;
    }
    let reality = &tls["reality"];
    if reality["enabled"] == json!(true) {
        q.push(("security".into(), "reality".into()));
        q.push((
            "pbk".into(),
            reality["public_key"].as_str().unwrap_or_default().into(),
        ));
        if let Some(sid) = reality["short_id"].as_str().filter(|s| !s.is_empty()) {
            q.push(("sid".into(), sid.into()));
        }
    } else {
        q.push(("security".into(), "tls".into()));
    }
    if let Some(sni) = tls["server_name"].as_str().filter(|s| !s.is_empty()) {
        q.push(("sni".into(), sni.into()));
    }
    if let Some(fp) = tls["utls"]["fingerprint"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        q.push(("fp".into(), fp.into()));
    }
    if let Some(alpn) = tls["alpn"].as_array() {
        let list: Vec<&str> = alpn.iter().filter_map(Value::as_str).collect();
        if !list.is_empty() {
            q.push(("alpn".into(), list.join(",")));
        }
    }
    if tls["insecure"] == json!(true) {
        q.push(("allowInsecure".into(), "1".into()));
    }
    q
}

fn stream_query(o: &Value) -> Vec<(String, String)> {
    let mut q = tls_query(o);
    let t = &o["transport"];
    let kind = t["type"].as_str().unwrap_or("tcp");
    q.push(("type".into(), kind.into()));
    match kind {
        "ws" | "httpupgrade" => {
            q.push(("path".into(), t["path"].as_str().unwrap_or("/").into()));
            if let Some(host) = t["headers"]["Host"].as_str().or_else(|| t["host"].as_str()) {
                q.push(("host".into(), host.into()));
            }
        }
        "grpc" => q.push((
            "serviceName".into(),
            t["service_name"].as_str().unwrap_or_default().into(),
        )),
        "http" => {
            q.push(("path".into(), t["path"].as_str().unwrap_or("/").into()));
            if let Some(hosts) = t["host"].as_array() {
                let list: Vec<&str> = hosts.iter().filter_map(Value::as_str).collect();
                q.push(("host".into(), list.join(",")));
            }
        }
        "xhttp" => {
            q.push(("path".into(), t["path"].as_str().unwrap_or("/").into()));
            q.push(("mode".into(), t["mode"].as_str().unwrap_or("auto").into()));
            if let Some(host) = t["host"].as_str() {
                q.push(("host".into(), host.into()));
            }
        }
        _ => {}
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link;

    /// Link → profile → link → profile: outbound and name must match.
    fn round_trip(original: &str) {
        let first = link::parse(original).unwrap();
        let regenerated = to_link(&first).unwrap();
        let second = link::parse(&regenerated).unwrap();
        assert_eq!(
            first.outbound, second.outbound,
            "\nисходная:  {original}\nсобранная: {regenerated}"
        );
        assert_eq!(first.name, second.name);
    }

    #[test]
    fn vless_reality_round_trip() {
        round_trip(
            "vless://uuid@de.example:443?security=reality&sni=de.example&fp=firefox\
             &pbk=KEY&sid=ba15&flow=xtls-rprx-vision&type=tcp#%F0%9F%87%A9%F0%9F%87%AA%20DE",
        );
    }

    #[test]
    fn vless_ws_round_trip() {
        round_trip("vless://uuid@a.example:443?type=ws&path=%2Fws&host=cdn.example&security=tls&fp=chrome#ws");
    }

    #[test]
    fn trojan_grpc_round_trip() {
        round_trip(
            "trojan://p%40ss@t.example:443?type=grpc&serviceName=svc&sni=t.example&fp=chrome#tr",
        );
    }

    #[test]
    fn shadowsocks_round_trip() {
        round_trip(&format!(
            "ss://{}@ss.example:8388#ss",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("aes-256-gcm:sec%ret")
        ));
    }

    #[test]
    fn hysteria2_round_trip() {
        round_trip(
            "hysteria2://pw@h.example:443?obfs=salamander&obfs-password=zzz&sni=h.example#hy",
        );
    }

    #[test]
    fn tuic_round_trip() {
        round_trip("tuic://uuid:pass@t.example:443?congestion_control=bbr&udp_relay_mode=native&sni=t.example#tu");
    }

    #[test]
    fn vmess_round_trip() {
        let original = link::parse(&format!(
            "vmess://{}",
            STANDARD.encode(
                r#"{"v":"2","ps":"tokyo","add":"jp.example","port":443,"id":"UUID","aid":0,
                    "net":"ws","path":"/p","host":"cdn.example","tls":"tls","sni":"cdn.example","scy":"auto"}"#
            )
        ))
        .unwrap();
        let again = link::parse(&to_link(&original).unwrap()).unwrap();
        assert_eq!(original.outbound, again.outbound);
        assert_eq!(again.name, "tokyo");
    }

    #[test]
    fn naive_round_trip() {
        round_trip("naive+https://user:pass@n.example:443?uot=1&sni=n.example#naive");
        round_trip("naive+quic://user:pass@n.example:443?congestion_control=bbr&sni=n.example#nq");
    }

    #[test]
    fn names_with_hash_and_ampersand_survive() {
        let mut p = link::parse("trojan://pw@t.example:443#x").unwrap();
        p.name = "NL #2 ⚡️ Torrent & Games".into();
        let again = link::parse(&to_link(&p).unwrap()).unwrap();
        assert_eq!(again.name, "NL #2 ⚡️ Torrent & Games");
    }

    #[test]
    fn ipv6_is_bracketed_in_link() {
        let p = link::parse("trojan://pw@[2001:db8::1]:443#v6").unwrap();
        assert!(to_link(&p).unwrap().contains("@[2001:db8::1]:443"));
    }

    #[test]
    fn unknown_protocol_is_reported() {
        let p =
            Profile::from_outbound(json!({"type": "wireguard", "server": "a", "server_port": 1}))
                .unwrap();
        assert!(to_link(&p).is_err());
    }
}
