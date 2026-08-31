//! Bridge to the Xray core.
//!
//! XHTTP is supported only by Xray, while all inbounds, DNS, and routing live in sing-box.
//! Such a profile is therefore executed as follows: sing-box connects to a local
//! Xray SOCKS inbound, which then connects to the server. The port is bound to loopback and
//! protected by a username and password; otherwise any process on the system would get
//! an open proxy.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::profile::Profile;
use crate::settings::Settings;

/// A profile moved to Xray, together with the bridge parameters.
#[derive(Debug, Clone)]
pub struct Bridge {
    /// The tag under which the profile is visible in sing-box and core responses.
    pub tag: String,
    pub port: u16,
    pub auth: String,
    /// Ready outbound in Xray format.
    pub outbound: Value,
}

impl Bridge {
    pub fn reserve(profile: &Profile, tag: &str) -> Result<Self> {
        Ok(Self {
            tag: tag.to_string(),
            port: free_port()?,
            auth: Uuid::new_v4().simple().to_string(),
            outbound: to_xray_outbound(profile, tag)?,
        })
    }

    /// How this profile appears to sing-box.
    pub fn socks_outbound(&self) -> Value {
        json!({
            "type": "socks",
            "tag": self.tag,
            "server": "127.0.0.1",
            "server_port": self.port,
            "version": "5",
            "username": self.auth,
            "password": self.auth,
        })
    }

    fn inbound_tag(&self) -> String {
        format!("{}-in", self.tag)
    }

    fn xray_inbound(&self) -> Value {
        json!({
            "tag": self.inbound_tag(),
            "protocol": "socks",
            "listen": "127.0.0.1",
            "port": self.port,
            "settings": {
                "auth": "password",
                "accounts": [{"user": self.auth, "pass": self.auth}],
                "udp": true,
                "address": "127.0.0.1",
            },
        })
    }
}

/// A free loopback port. There is a window between releasing it and Xray taking it,
/// as with the original Throne, and in practice the port is acquired by us in time.
fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("не удалось занять локальный порт под мост в Xray")?;
    Ok(listener.local_addr()?.port())
}

/// Complete Xray configuration for a set of bridges: each has its own inbound, outbound, and
/// rule connecting them.
pub fn build_config(bridges: &[Bridge], settings: &Settings) -> Result<String> {
    if bridges.is_empty() {
        bail!("нет ни одного профиля для Xray");
    }
    let config = json!({
        "log": {"loglevel": xray_log_level(&settings.log_level)},
        "inbounds": bridges.iter().map(Bridge::xray_inbound).collect::<Vec<_>>(),
        "outbounds": bridges.iter().map(|b| b.outbound.clone()).collect::<Vec<_>>(),
        "routing": {
            "rules": bridges
                .iter()
                .map(|b| json!({
                    "type": "field",
                    "inboundTag": [b.inbound_tag()],
                    "outboundTag": b.tag,
                }))
                .collect::<Vec<_>>(),
        },
    });
    Ok(serde_json::to_string_pretty(&config)?)
}

fn xray_log_level(level: &str) -> &'static str {
    match level {
        "trace" | "debug" => "debug",
        "info" => "info",
        "error" | "fatal" | "panic" => "error",
        _ => "warning",
    }
}

/// Converts an outbound to Xray format.
///
/// A profile already stored in Xray format is returned as is; rewriting it
/// is forbidden: extensions such as `xmux` and `downloadSettings` have no
/// sing-box representation and would be lost in a round trip. Everything else is converted from
/// sing-box format; vless is supported because it is what arrives with XHTTP.
pub fn to_xray_outbound(profile: &Profile, tag: &str) -> Result<Value> {
    if profile.is_xray_native() {
        let mut out = profile.outbound.clone();
        out["tag"] = json!(tag);
        return Ok(out);
    }

    let o = profile
        .outbound
        .as_object()
        .ok_or_else(|| anyhow!("outbound профиля не является объектом"))?;

    let server = o
        .get("server")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("в профиле нет адреса сервера"))?;
    let port = o
        .get("server_port")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("в профиле нет порта сервера"))?;
    let uuid = o
        .get("uuid")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("в профиле нет UUID"))?;

    let mut user = json!({"id": uuid, "encryption": "none"});
    if let Some(flow) = o
        .get("flow")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
    {
        user["flow"] = json!(flow);
    }

    Ok(json!({
        "tag": tag,
        "protocol": "vless",
        "settings": {
            "vnext": [{"address": server, "port": port, "users": [user]}],
        },
        "streamSettings": stream_settings(o),
    }))
}

fn stream_settings(o: &Map<String, Value>) -> Value {
    let transport = o.get("transport");
    let network = transport
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("tcp");

    let mut ss = Map::new();
    ss.insert("network".into(), json!(network));

    match network {
        "xhttp" => {
            let t = transport.unwrap();
            let mut x = json!({
                "path": t.get("path").and_then(Value::as_str).unwrap_or("/"),
                "mode": t.get("mode").and_then(Value::as_str).unwrap_or("auto"),
            });
            if let Some(host) = t
                .get("host")
                .and_then(Value::as_str)
                .filter(|h| !h.is_empty())
            {
                x["host"] = json!(host);
            }
            if let Some(extra) = t.get("extra") {
                x["extra"] = extra.clone();
            }
            ss.insert("xhttpSettings".into(), x);
        }
        "ws" => {
            let t = transport.unwrap();
            let mut w = json!({"path": t.get("path").and_then(Value::as_str).unwrap_or("/")});
            if let Some(headers) = t.get("headers") {
                w["headers"] = headers.clone();
            }
            ss.insert("wsSettings".into(), w);
        }
        "grpc" => {
            let t = transport.unwrap();
            ss.insert(
                "grpcSettings".into(),
                json!({
                    "serviceName": t.get("service_name").and_then(Value::as_str).unwrap_or(""),
                }),
            );
        }
        "http" => {
            let t = transport.unwrap();
            let mut h = json!({"path": t.get("path").and_then(Value::as_str).unwrap_or("/")});
            if let Some(host) = t.get("host") {
                h["host"] = host.clone();
            }
            ss.insert("httpSettings".into(), h);
        }
        _ => {}
    }

    let Some(tls) = o.get("tls").filter(|t| t["enabled"] == json!(true)) else {
        ss.insert("security".into(), json!("none"));
        return Value::Object(ss);
    };

    let sni = tls.get("server_name").and_then(Value::as_str).unwrap_or("");
    let fingerprint = tls
        .get("utls")
        .and_then(|u| u.get("fingerprint"))
        .and_then(Value::as_str)
        .unwrap_or("chrome");

    if let Some(reality) = tls.get("reality").filter(|r| r["enabled"] == json!(true)) {
        ss.insert("security".into(), json!("reality"));
        ss.insert(
            "realitySettings".into(),
            json!({
                "serverName": sni,
                "fingerprint": fingerprint,
                "publicKey": reality.get("public_key").and_then(Value::as_str).unwrap_or(""),
                "shortId": reality.get("short_id").and_then(Value::as_str).unwrap_or(""),
                "spiderX": "",
            }),
        );
    } else {
        ss.insert("security".into(), json!("tls"));
        let mut t = json!({"serverName": sni, "fingerprint": fingerprint});
        if tls.get("insecure") == Some(&json!(true)) {
            t["allowInsecure"] = json!(true);
        }
        if let Some(alpn) = tls.get("alpn") {
            t["alpn"] = alpn.clone();
        }
        ss.insert("tlsSettings".into(), t);
    }

    Value::Object(ss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link;

    fn xhttp_profile() -> Profile {
        link::parse(
            "vless://uuid@x.example:443?type=xhttp&security=reality&pbk=KEY&sid=aa&fp=chrome\
             &sni=x.example&path=%2Fpath&mode=packet-up#x",
        )
        .unwrap()
    }

    #[test]
    fn xhttp_reality_translates_to_xray() {
        let out = to_xray_outbound(&xhttp_profile(), "proxy").unwrap();
        assert_eq!(out["protocol"], "vless");
        assert_eq!(out["settings"]["vnext"][0]["address"], "x.example");
        assert_eq!(out["settings"]["vnext"][0]["users"][0]["id"], "uuid");
        let ss = &out["streamSettings"];
        assert_eq!(ss["network"], "xhttp");
        assert_eq!(ss["security"], "reality");
        assert_eq!(ss["realitySettings"]["publicKey"], "KEY");
        assert_eq!(ss["realitySettings"]["fingerprint"], "chrome");
        assert_eq!(ss["xhttpSettings"]["path"], "/path");
        assert_eq!(ss["xhttpSettings"]["mode"], "packet-up");
    }

    #[test]
    fn bridge_wires_singbox_to_xray_on_one_port() {
        let bridge = Bridge::reserve(&xhttp_profile(), "proxy").unwrap();
        let socks = bridge.socks_outbound();
        assert_eq!(socks["type"], "socks");
        assert_eq!(socks["server"], "127.0.0.1");
        assert_eq!(socks["server_port"], bridge.port);
        assert_eq!(socks["username"], bridge.auth);

        let config: Value =
            serde_json::from_str(&build_config(&[bridge.clone()], &Settings::default()).unwrap())
                .unwrap();
        assert_eq!(config["inbounds"][0]["port"], bridge.port);
        assert_eq!(
            config["inbounds"][0]["settings"]["accounts"][0]["user"],
            bridge.auth
        );
        assert_eq!(config["routing"]["rules"][0]["inboundTag"][0], "proxy-in");
        assert_eq!(config["routing"]["rules"][0]["outboundTag"], "proxy");
    }

    #[test]
    fn each_bridge_gets_its_own_port() {
        let a = Bridge::reserve(&xhttp_profile(), "p-0").unwrap();
        let b = Bridge::reserve(&xhttp_profile(), "p-1").unwrap();
        assert_ne!(a.port, b.port);
        assert_ne!(a.auth, b.auth);
    }

    #[test]
    fn plain_tls_without_reality() {
        let p = link::parse(
            "vless://uuid@w.example:443?type=xhttp&security=tls&sni=w.example&alpn=h2#w",
        )
        .unwrap();
        let ss = to_xray_outbound(&p, "proxy").unwrap()["streamSettings"].clone();
        assert_eq!(ss["security"], "tls");
        assert_eq!(ss["tlsSettings"]["serverName"], "w.example");
        assert_eq!(ss["tlsSettings"]["alpn"][0], "h2");
    }

    #[test]
    fn native_xray_outbound_passes_through_untouched() {
        let p = Profile::from_outbound(json!({
            "protocol": "vless",
            "settings": {"address": "h2.example", "port": 443, "id": "u", "encryption": "none"},
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "xhttpSettings": {"extra": {"xmux": {"maxConnections": "1-1"}}}
            }
        }))
        .unwrap();
        let out = to_xray_outbound(&p, "proxy").unwrap();
        assert_eq!(out["tag"], "proxy");
        // An extension absent from sing-box format must survive.
        assert_eq!(
            out["streamSettings"]["xhttpSettings"]["extra"]["xmux"]["maxConnections"],
            "1-1"
        );
    }

    #[test]
    fn profile_without_uuid_is_rejected() {
        let p = Profile::from_outbound(json!({"type": "vless", "server": "a", "server_port": 1}))
            .unwrap();
        assert!(to_xray_outbound(&p, "proxy").is_err());
    }
}
