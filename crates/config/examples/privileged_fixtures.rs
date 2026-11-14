//! Reproduce synthetic policy goldens without user data or external requests:
//! PRIVILEGED_FIXTURE_DIR=/tmp/opencode/privileged-fixtures cargo run -p nolurix-config --example privileged_fixtures
//! Copy the resulting JSON into core/testdata/privileged after review.
use anyhow::{ensure, Result};
use nolurix_config::{
    generate, link,
    settings::{ProxyMode, Settings},
};
use serde_json::{json, Value};
use std::{env, fs, path::PathBuf};

fn save(dir: &std::path::Path, name: &str, g: generate::GeneratedConfig) -> Result<()> {
    let mut core: Value = serde_json::from_str(&g.core_config)?;
    let xray = if g.need_xray {
        // generate() reserves a loopback bridge port and random credentials;
        // normalize just these synthetic bridge values for reproducible goldens.
        let mut x: Value = serde_json::from_str(&g.xray_config)?;
        for o in core["outbounds"].as_array_mut().unwrap() {
            if o["type"] == "socks" && o["server"] == "127.0.0.1" {
                o["server_port"] = json!(22222);
                o["username"] = json!("synthetic-bridge-user");
                o["password"] = json!("synthetic-bridge-password");
            }
        }
        for i in x["inbounds"].as_array_mut().unwrap() {
            i["port"] = json!(22222);
            i["settings"]["accounts"][0] =
                json!({"user":"synthetic-bridge-user","pass":"synthetic-bridge-password"});
        }
        x
    } else {
        Value::Null
    };
    let fixture = json!({"core_config":core,"xray_config":xray});
    fs::write(
        dir.join(format!("{name}.json")),
        format!("{}\n", serde_json::to_string_pretty(&fixture)?),
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let dir = PathBuf::from(env::var("PRIVILEGED_FIXTURE_DIR")?);
    ensure!(
        dir.is_absolute() && dir.starts_with("/tmp/opencode"),
        "fixture directory must be under /tmp/opencode"
    );
    fs::create_dir_all(&dir)?;
    let mut settings = Settings::default();
    settings.mode = ProxyMode::Vpn;
    settings.cache_file = "/synthetic/user/cache.db".into();
    settings.test_url = "https://test.example/generate_204".into();
    let links = [
        ("vless-reality", "vless://00000000-0000-0000-0000-000000000001@proxy.example:443?security=reality&sni=proxy.example&pbk=SYNTHETIC_KEY&sid=aa&flow=xtls-rprx-vision"),
        ("vless-websocket", "vless://00000000-0000-0000-0000-000000000001@proxy.example:443?security=tls&type=ws&host=cdn.example&path=%2Fws%3Fed%3D2048"),
        ("vless-xhttp", "vless://00000000-0000-0000-0000-000000000001@proxy.example:443?security=reality&sni=proxy.example&pbk=SYNTHETIC_KEY&sid=aa&type=xhttp&path=%2Fsynthetic-path&mode=packet-up"),
        ("hysteria2", "hysteria2://synthetic-password@proxy.example:443?obfs=salamander&obfs-password=synthetic-obfs&upmbps=50&downmbps=100"),
        ("tuic", "tuic://00000000-0000-0000-0000-000000000001:synthetic-password@proxy.example:443?congestion_control=bbr"),
        ("anytls", "anytls://synthetic-password@proxy.example:443"),
        ("naive", "naive+https://synthetic-user:synthetic-password@proxy.example:443"),
        ("naive-quic", "naive+quic://synthetic-user:synthetic-password@proxy.example:443?uot=1&congestion_control=bbr"),
        ("socks", "socks5://synthetic-user:synthetic-password@proxy.example:1080"),
        ("http", "https://synthetic-user:synthetic-password@proxy.example:443"),
        ("trojan", "trojan://synthetic-password@proxy.example:443?security=tls&type=grpc&serviceName=synthetic-service"),
        ("shadowsocks", "ss://YWVzLTEyOC1nY206c3ludGhldGljLXBhc3N3b3Jk@proxy.example:443"),
    ];
    for (name, uri) in links {
        save(
            &dir,
            name,
            generate::generate(&link::parse(uri)?, &settings)?,
        )?;
    }
    let vmess = nolurix_config::profile::Profile::from_outbound(
        json!({"type":"vmess","server":"proxy.example","server_port":443,"uuid":"00000000-0000-0000-0000-000000000001","alter_id":0,"security":"auto","packet_encoding":"xudp","transport":{"type":"ws","path":"/synthetic-ws"}}),
    )?;
    settings.mux_enabled = true;
    save(&dir, "vmess-mux", generate::generate(&vmess, &settings)?)?;
    settings.mux_enabled = false;
    let native = nolurix_config::profile::Profile::from_outbound(
        json!({"protocol":"vless","settings":{"vnext":[{"address":"proxy.example","port":443,"users":[{"id":"00000000-0000-0000-0000-000000000001","encryption":"none"}]}]},"streamSettings":{"network":"xhttp","security":"tls","tlsSettings":{"serverName":"proxy.example","fingerprint":"chrome"},"xhttpSettings":{"path":"/synthetic-native","mode":"auto","extra":{"xmux":{"maxConnections":"1-1"},"downloadSettings":{"address":"download.example","port":443,"network":"xhttp","security":"tls","tlsSettings":{"serverName":"download.example"},"xhttpSettings":{"path":"/synthetic-download","mode":"auto"}}}}}}),
    )?;
    save(
        &dir,
        "native-vless-xhttp",
        generate::generate(&native, &settings)?,
    )?;
    let profiles = [link::parse(links[0].1)?, link::parse(links[3].1)?];
    settings.auto_balance = true;
    settings.fakedns = true;
    settings.tun_ipv6 = true;
    settings.dns_remote = "https://dns.example/synthetic-dns-query".into();
    settings.custom_route_rules = json!([{"type":"logical","mode":"or","rules":[{"domain_suffix":["synthetic.example"]},{"process_path":["/synthetic/application"]}],"action":"route","outbound":"direct"}]).to_string();
    save(
        &dir,
        "auto-routing-fakedns",
        generate::generate_auto(&profiles, &settings)?,
    )?;
    println!("wrote synthetic fixtures to {}", dir.display());
    Ok(())
}
