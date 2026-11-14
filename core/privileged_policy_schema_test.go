package main

import (
	"context"
	"encoding/json"
	"github.com/nalart11/Nolurix/core/gen"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPrivilegedPolicyRustGeneratedGoldens(t *testing.T) {
	enablePrivilegedWorkerPolicy()
	t.Cleanup(func() { privilegedWorker.Store(false) })
	paths, err := filepath.Glob("testdata/privileged/*.json")
	if err != nil || len(paths) != 15 {
		t.Fatalf("expected 15 generated fixtures: %d, %v", len(paths), err)
	}
	for _, path := range paths {
		t.Run(filepath.Base(path), func(t *testing.T) {
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			var fixture struct {
				Core json.RawMessage `json:"core_config"`
				Xray json.RawMessage `json:"xray_config"`
			}
			if err := json.Unmarshal(data, &fixture); err != nil {
				t.Fatal(err)
			}
			req := &gen.LoadConfigReq{CoreConfig: To(string(fixture.Core))}
			if string(fixture.Xray) != "null" {
				req.NeedXray = To(true)
				req.XrayConfig = To(string(fixture.Xray))
			}
			before := req.GetCoreConfig()
			got, err := privilegedStartRequest(req)
			if err != nil {
				t.Fatal(err)
			}
			if req.GetCoreConfig() != before || !strings.Contains(before, "/synthetic/user/cache.db") {
				t.Fatal("caller config modified or missing synthetic cache")
			}
			var normalized map[string]any
			if err := json.Unmarshal([]byte(got.GetCoreConfig()), &normalized); err != nil {
				t.Fatal(err)
			}
			cache := normalized["experimental"].(map[string]any)["cache_file"].(map[string]any)
			if len(cache) != 1 || cache["enabled"] != false {
				t.Fatal("privileged disk cache not disabled")
			}
		})
	}
}

func TestPrivilegedPolicySchemaErrorsRedacted(t *testing.T) {
	enablePrivilegedWorkerPolicy()
	t.Cleanup(func() { privilegedWorker.Store(false) })
	for _, raw := range []string{
		`{"log":{"SECRET_UNKNOWN_KEY":true}}`,
		`{"outbounds":[{"type":"http","headers":{"SECRET_HEADER_KEY":{"nested":"value"}}}]}`,
	} {
		_, err := privilegedStartRequest(&gen.LoadConfigReq{CoreConfig: To(raw)})
		if err == nil || strings.Contains(err.Error(), "SECRET") {
			t.Fatalf("missing or unredacted error: %v", err)
		}
	}
}

// Representative exact object shapes from crates/config generate.rs, link.rs,
// and xray.rs. Paths here are network URL paths, not local file references.
const generatedBoxV1 = `{"log":{"level":"info","timestamp":true},"inbounds":[{"type":"mixed","tag":"mixed-in","listen":"127.0.0.1","listen_port":2080},{"type":"tun","tag":"tun-in","address":["172.19.0.1/30"],"mtu":9000,"auto_route":true,"auto_redirect":true,"strict_route":true,"stack":"mixed"}],"outbounds":[{"type":"direct","tag":"direct"}],"dns":{"servers":[{"type":"https","server":"dns.example","path":"/dns-query","tag":"dns-remote","detour":"proxy","domain_resolver":"dns-local"},{"type":"local","tag":"dns-local"}],"rules":[{"action":"route","server":"dns-remote","strategy":"prefer_ipv4"}]},"route":{"rules":[{"action":"sniff"},{"protocol":"dns","action":"hijack-dns"},{"type":"logical","mode":"or","rules":[{"domain_suffix":["example.org"]}],"action":"route","outbound":"direct"}],"rule_set":[{"type":"remote","tag":"private","format":"binary","url":"https://example.org/private.srs","download_detour":"proxy"}],"final":"proxy","default_domain_resolver":{"server":"dns-local","strategy":"prefer_ipv4"},"find_process":true,"auto_detect_interface":true},"experimental":{"cache_file":{"enabled":true,"path":"/user/cache.db","store_fakeip":true},"clash_api":{"default_mode":""}}}`
const generatedXrayV1 = `{"log":{"loglevel":"warning"},"inbounds":[{"tag":"proxy-in","protocol":"socks","listen":"127.0.0.1","port":2222,"settings":{"auth":"password","accounts":[{"user":"u","pass":"p"}],"udp":true,"address":"127.0.0.1"}}],"outbounds":[{"tag":"proxy","protocol":"vless","settings":{"vnext":[{"address":"x.example","port":443,"users":[{"id":"uuid","encryption":"none"}]}]},"streamSettings":{"network":"xhttp","security":"reality","realitySettings":{"serverName":"x.example","fingerprint":"chrome","publicKey":"KEY","shortId":"aa","spiderX":""},"xhttpSettings":{"path":"/path","mode":"packet-up","extra":{"xmux":{"maxConcurrency":"16-32"}}}}}],"routing":{"rules":[{"type":"field","inboundTag":["proxy-in"],"outboundTag":"proxy"}]}}`

func TestPrivilegedPolicySchemaGenerated(t *testing.T) {
	enablePrivilegedWorkerPolicy()
	t.Cleanup(func() { privilegedWorker.Store(false) })
	protocols := []string{
		`{"type":"vless","uuid":"uuid","flow":"xtls-rprx-vision","packet_encoding":"xudp","tls":{"enabled":true,"reality":{"enabled":true,"public_key":"KEY","short_id":"aa"},"utls":{"enabled":true,"fingerprint":"chrome"}}}`,
		`{"type":"vmess","uuid":"uuid","alter_id":0,"security":"auto","transport":{"type":"ws","path":"/ws?ed=1","headers":{"Host":"cdn.example"}},"multiplex":{"enabled":true,"protocol":"smux","max_streams":8}}`,
		`{"type":"trojan","password":"p","tls":{"enabled":true,"certificate":["inline PEM"],"key":["inline PEM"]},"transport":{"type":"http","path":"/http","host":["cdn.example"]}}`,
		`{"type":"shadowsocks","method":"aes-128-gcm","password":"p"}`,
		`{"type":"hysteria2","password":"p","up_mbps":50,"down_mbps":100,"obfs":{"type":"salamander","password":"p"},"tls":{"enabled":true}}`,
		`{"type":"tuic","uuid":"uuid","password":"p","congestion_control":"bbr","udp_relay_mode":"native","tls":{"enabled":true}}`,
		`{"type":"anytls","password":"p","tls":{"enabled":true}}`,
		`{"type":"naive","username":"u","password":"p","quic":true,"quic_congestion_control":"bbr","udp_over_tcp":true,"tls":{"enabled":true}}`,
		`{"type":"socks","version":"5","username":"u","password":"p"}`,
		`{"type":"http","username":"u","password":"p","path":"/proxy","tls":{"enabled":true}}`,
		`{"type":"auto-selector","outbounds":["p-0"],"url":"https://example.org/test","interval":"30s","tolerance":50,"timeout":"1000ms","concurrency":4,"dial_retries":2,"warm":[{"tag":"p-0","rtt":40,"age":10}],"balance":true,"balance_mode":"rotate"}`,
	}
	for _, raw := range protocols {
		var b map[string]any
		_ = json.Unmarshal([]byte(generatedBoxV1), &b)
		var o map[string]any
		_ = json.Unmarshal([]byte(raw), &o)
		o["tag"] = "proxy"
		if o["type"] != "auto-selector" {
			o["server"] = "a.example"
			o["server_port"] = float64(443)
		}
		b["outbounds"] = append(b["outbounds"].([]any), o)
		data, _ := json.Marshal(b)
		req := &gen.LoadConfigReq{CoreConfig: To(string(data)), NeedXray: To(true), XrayConfig: To(generatedXrayV1)}
		out, err := privilegedStartRequest(req)
		if err != nil {
			t.Fatalf("%s: %v", o["type"], err)
		}
		if strings.Contains(out.GetCoreConfig(), "/user/cache.db") || !strings.Contains(out.GetCoreConfig(), `"cache_file":{"enabled":false}`) {
			t.Fatal("cache not disabled")
		}
		if !strings.Contains(req.GetCoreConfig(), "/user/cache.db") {
			t.Fatal("mutated original request")
		}
	}
}

func TestPrivilegedPolicySchemaMalicious(t *testing.T) {
	enablePrivilegedWorkerPolicy()
	t.Cleanup(func() { privilegedWorker.Store(false) })
	for _, raw := range []string{
		`{"outbounds":[{"type":"tor","executable_path":"/tmp/pwn"}]}`,
		`{"outbounds":[{"type":"ssh","private_key_path":"/etc/shadow"}]}`,
		`{"outbounds":[{"type":"vless","tls":{"certificate_path":"/etc/shadow"}}]}`,
		`{"outbounds":[{"type":"vless","tls":{"ech":{"config_path":"/etc/shadow"}}}]}`,
		`{"outbounds":[{"type":"shadowsocks","plugin":"/tmp/pwn"}]}`,
		`{"route":{"rule_set":[{"type":"local","path":"/etc/shadow"}]}}`,
		`{"log":{"output":"/etc/cron.d/pwn"}}`,
		`{"experimental":{"clash_api":{"external_controller":"0.0.0.0:9090"}}}`,
		`{"services":[{"type":"ssm-api"}]}`,
		`{"outbounds":[{"type":"vless","transport":{"type":"ws","headers":{"Host":{"path":"/etc/shadow"}}}}]}`,
		`{"outbounds":[{"type":"vless","tls":{"enabled":"true"}}]}`,
	} {
		req := &gen.LoadConfigReq{CoreConfig: To(raw)}
		if out, err := (&server{}).Start(context.Background(), req); err != nil || out.GetError() == "" {
			t.Fatalf("not denied before construction: %s (%v)", raw, err)
		}
	}
	for _, addition := range []string{`"certificateFile":"/etc/shadow"`, `"keyFile":"/etc/shadow"`, `"unknown":{"path":"/tmp/pwn"}`} {
		x := strings.Replace(generatedXrayV1, `"publicKey":"KEY"`, `"publicKey":"KEY",`+addition, 1)
		if _, err := privilegedStartRequest(&gen.LoadConfigReq{CoreConfig: To(generatedBoxV1), NeedXray: To(true), XrayConfig: To(x)}); err == nil {
			t.Fatal("accepted nested Xray file/unknown field")
		}
	}
	for _, req := range []*gen.LoadConfigReq{
		{CoreConfig: To(generatedBoxV1), NeedExtraProcess: To(true)},
		{CoreConfig: To(generatedBoxV1), ExtraProcessPath: To("/tmp/pwn")},
		{CoreConfig: To(generatedBoxV1), XrayFullConfigs: []string{`{}`}},
	} {
		if _, err := privilegedStartRequest(req); err == nil {
			t.Fatal("accepted opaque process/config")
		}
	}
}

func TestPrivilegedPolicyOrdinaryUnrestricted(t *testing.T) {
	privilegedWorker.Store(false)
	req := &gen.LoadConfigReq{CoreConfig: To(`{"unknown":{"file":"/user/config"}}`), NeedExtraProcess: To(true)}
	got, err := privilegedStartRequest(req)
	if err != nil || got != req {
		t.Fatalf("ordinary request changed: %v", err)
	}
}
