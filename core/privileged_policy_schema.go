package main

import (
	"ThroneGtkCore/gen"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"google.golang.org/protobuf/proto"
)

// Schema v1 deliberately describes GUI-generated network configurations, not
// the complete upstream configuration languages. New fields require review.
// s,b,n are scalar types; [] is an array; = is a closed string enum. Every
// object rejects unknown fields, recursively, including native Xray outbounds.
var privilegedSchemasV1 = map[string]string{
	"box":                   "log:log dns:dns inbounds:[]in outbounds:[]out route:route experimental:experimental",
	"log":                   "level:s timestamp:b disabled:b",
	"experimental":          "cache_file:cache clash_api:clash",
	"cache":                 "enabled:b path:s store_fakeip:b",
	"clash":                 "default_mode:s",
	"in/mixed":              "type:=mixed tag:s listen:s listen_port:n users:[]auth",
	"auth":                  "username:s password:s",
	"in/tun":                "type:=tun tag:s address:[]s mtu:n auto_route:b auto_redirect:b strict_route:b stack:=mixed|system|gvisor interface_name:s",
	"out/direct":            "type:=direct tag:s",
	"out/vless":             "uuid:s flow:s packet_encoding:s tls:tls transport:transport multiplex:mux",
	"out/vmess":             "uuid:s alter_id:n security:s packet_encoding:s tls:tls transport:transport multiplex:mux",
	"out/trojan":            "password:s tls:tls transport:transport multiplex:mux",
	"out/shadowsocks":       "method:s password:s multiplex:mux udp_over_tcp:b",
	"out/hysteria2":         "password:s up_mbps:n down_mbps:n obfs:obfs tls:tls",
	"obfs":                  "type:=salamander password:s",
	"out/tuic":              "uuid:s password:s congestion_control:s udp_relay_mode:s zero_rtt_handshake:b heartbeat:s tls:tls",
	"out/anytls":            "password:s idle_session_check_interval:s idle_session_timeout:s min_idle_session:n tls:tls",
	"out/naive":             "username:s password:s quic:b quic_congestion_control:s udp_over_tcp:b tls:tls",
	"out/socks":             "version:=4|4a|5 username:s password:s udp_over_tcp:b",
	"out/http":              "username:s password:s tls:tls path:s headers:headers",
	"out/selector":          "type:=selector tag:s outbounds:[]s default:s interrupt_exist_connections:b",
	"out/auto-selector":     "type:=auto-selector tag:s outbounds:[]s url:s interval:s tolerance:n timeout:s concurrency:n dial_retries:n warm:[]warm balance:b balance_mode:=rotate|random",
	"warm":                  "tag:s rtt:n age:n",
	"mux":                   "enabled:b protocol:=smux|yamux|h2mux max_connections:n min_streams:n max_streams:n padding:b brutal:brutal",
	"brutal":                "enabled:b up_mbps:n down_mbps:n",
	"tls":                   "enabled:b disable_sni:b server_name:s insecure:b alpn:[]s min_version:s max_version:s cipher_suites:[]s certificate:[]s key:[]s utls:utls reality:reality ech:ech",
	"utls":                  "enabled:b fingerprint:s",
	"reality":               "enabled:b public_key:s short_id:s",
	"ech":                   "enabled:b config:[]s",
	"transport/ws":          "type:=ws path:s headers:headers max_early_data:n early_data_header_name:s",
	"transport/http":        "type:=http host:[]s path:s method:s headers:headers idle_timeout:s ping_timeout:s",
	"transport/httpupgrade": "type:=httpupgrade host:s path:s headers:headers",
	"transport/grpc":        "type:=grpc service_name:s idle_timeout:s ping_timeout:s permit_without_stream:b",
	"dns":                   "servers:[]dnsserver rules:[]rule final:s strategy:s independent_cache:b disable_cache:b disable_expire:b reverse_mapping:b",
	"dnsserver/local":       "type:=local tag:s",
	"dnsserver/dhcp":        "type:=dhcp tag:s interface:s",
	"dnsserver/fakeip":      "type:=fakeip tag:s inet4_range:s inet6_range:s",
	"dnsserver/udp":         "",
	"dnsserver/tcp":         "",
	"dnsserver/tls":         "tls:tls",
	"dnsserver/quic":        "tls:tls",
	"dnsserver/https":       "path:s headers:headers tls:tls",
	"dnsserver/h3":          "path:s headers:headers tls:tls",
	"route":                 "rules:[]rule rule_set:[]ruleset final:s default_domain_resolver:resolver find_process:b auto_detect_interface:b",
	"resolver":              "server:s strategy:s",
	"ruleset":               "type:=remote tag:s format:=binary|source url:s download_detour:s update_interval:s",
	"rule":                  "type:=logical|default mode:=and|or rules:[]rule invert:b action:=route|reject|sniff|hijack-dns|resolve outbound:s server:s strategy:s inbound:strings protocol:strings network:strings domain:strings domain_suffix:strings domain_keyword:strings domain_regex:strings ip_cidr:strings source_ip_cidr:strings ip_is_private:b source_ip_is_private:b port:ports source_port:ports port_range:strings source_port_range:strings process_name:strings process_path:strings process_path_regex:strings rule_set:strings query_type:strings clash_mode:s",
	"xray":                  "log:xlog inbounds:[]xin outbounds:[]xout routing:xrouting",
	"xlog":                  "loglevel:=debug|info|warning|error|none",
	"xin":                   "tag:s protocol:=socks listen:=127.0.0.1|::1 port:n settings:xins",
	"xins":                  "auth:=password accounts:[]xauth udp:b address:=127.0.0.1|::1",
	"xauth":                 "user:s pass:s",
	"xout/vless":            "tag:s protocol:=vless settings:xvless streamSettings:stream mux:xmux",
	"xvless":                "vnext:[]xnext",
	"xnext":                 "address:s port:n users:[]xuser",
	"xuser":                 "id:s encryption:s flow:s level:n",
	"xmux":                  "enabled:b concurrency:n xudpConcurrency:n xudpProxyUDP443:=reject|allow|skip",
	"stream":                "address:s port:n network:=tcp|raw|xhttp|splithttp|ws|grpc|http security:=none|tls|reality tlsSettings:xtls realitySettings:xreality xhttpSettings:xhttp splithttpSettings:xhttp wsSettings:xws grpcSettings:xgrpc httpSettings:xhttptransport",
	"xtls":                  "serverName:s fingerprint:s allowInsecure:b alpn:[]s certificates:[]xcert pinnedPeerCertSha256:[]s",
	"xcert":                 "certificate:[]s key:[]s usage:=encipherment|verify|issue buildChain:b oneTimeLoading:b",
	"xreality":              "serverName:s fingerprint:s publicKey:s password:s shortId:s spiderX:s",
	"xws":                   "path:s host:s headers:headers maxEarlyData:n earlyDataHeaderName:s",
	"xgrpc":                 "serviceName:s multiMode:b idle_timeout:n health_check_timeout:n permit_without_stream:b initial_windows_size:n",
	"xhttptransport":        "path:s host:[]s",
	"xhttp":                 "path:s host:s mode:=auto|packet-up|stream-up|stream-one extra:xextra",
	"xextra":                "headers:headers xPaddingBytes:range noGRPCHeader:b noSSEHeader:b scMaxEachPostBytes:range scMinPostsIntervalMs:range scMaxBufferedPosts:n scStreamUpServerSecs:range xmux:xxmux downloadSettings:stream",
	"xxmux":                 "maxConcurrency:range maxConnections:range cMaxReuseTimes:range hMaxRequestTimes:range hMaxReusableSecs:range hKeepAlivePeriod:n",
	"xrouting":              "rules:[]xrule domainStrategy:=AsIs|IPIfNonMatch|IPOnDemand",
	"xrule":                 "type:=field inboundTag:[]s outboundTag:s",
}

func validatePrivilegedNode(v any, kind, at string, depth int) error {
	bad := func() error { return fmt.Errorf("privileged schema v1: unsupported field/type at %s (%s)", at, kind) }
	if depth > 64 {
		return bad()
	}
	if strings.HasPrefix(kind, "[]") {
		a, ok := v.([]any)
		if !ok {
			return bad()
		}
		for _, item := range a {
			if err := validatePrivilegedNode(item, kind[2:], at+"[]", depth+1); err != nil {
				return err
			}
		}
		return nil
	}
	if strings.HasPrefix(kind, "=") {
		s, ok := v.(string)
		if !ok {
			return bad()
		}
		for _, allowed := range strings.Split(kind[1:], "|") {
			if s == allowed {
				return nil
			}
		}
		return bad()
	}
	switch kind {
	case "s":
		if _, ok := v.(string); ok {
			return nil
		}
		return bad()
	case "b":
		if _, ok := v.(bool); ok {
			return nil
		}
		return bad()
	case "n":
		if n, ok := v.(float64); ok && n >= 0 && n == float64(uint64(n)) {
			return nil
		}
		return bad()
	case "range":
		if _, ok := v.(string); ok {
			return nil
		}
		return validatePrivilegedNode(v, "n", at, depth+1)
	case "strings":
		if _, ok := v.(string); ok {
			return nil
		}
		return validatePrivilegedNode(v, "[]s", at, depth+1)
	case "ports":
		if _, ok := v.(float64); ok {
			return validatePrivilegedNode(v, "n", at, depth+1)
		}
		return validatePrivilegedNode(v, "[]n", at, depth+1)
	case "headers":
		m, ok := v.(map[string]any)
		if !ok {
			return bad()
		}
		for _, value := range m {
			if err := validatePrivilegedNode(value, "strings", at+".<header>", depth+1); err != nil {
				return err
			}
		}
		return nil
	}
	m, ok := v.(map[string]any)
	if !ok {
		return bad()
	}
	base := kind
	switch kind {
	case "in", "out", "transport", "dnsserver":
		t, ok := m["type"].(string)
		if !ok {
			return bad()
		}
		kind += "/" + t
	case "xout":
		t, ok := m["protocol"].(string)
		if !ok {
			return bad()
		}
		kind += "/" + t
	}
	spec, ok := privilegedSchemasV1[kind]
	if !ok {
		return bad()
	}
	if base == "out" && kind != "out/direct" && kind != "out/selector" && kind != "out/auto-selector" {
		spec += " type:s tag:s server:s server_port:n network:s connect_timeout:s detour:s domain_resolver:strings"
	}
	if base == "dnsserver" && kind != "dnsserver/local" && kind != "dnsserver/dhcp" && kind != "dnsserver/fakeip" {
		spec += " type:s tag:s server:s server_port:n detour:s domain_resolver:s"
	}
	fields := map[string]string{}
	for _, field := range strings.Fields(spec) {
		pair := strings.SplitN(field, ":", 2)
		fields[pair[0]] = pair[1]
	}
	for key, value := range m {
		t, ok := fields[key]
		if !ok {
			return fmt.Errorf("privileged schema v1: unsupported field at %s", at)
		}
		if err := validatePrivilegedNode(value, t, at+"."+key, depth+1); err != nil {
			return err
		}
	}
	return nil
}

func privilegedStartRequest(in *gen.LoadConfigReq) (*gen.LoadConfigReq, error) {
	if !privilegedWorker.Load() {
		return in, nil
	}
	if in == nil {
		return nil, fmt.Errorf("privileged worker: missing Start request")
	}
	if in.GetNeedExtraProcess() || in.GetExtraProcessPath() != "" || in.GetExtraProcessConf() != "" || in.GetExtraProcessArgs() != "" || len(in.GetXrayFullConfigs()) != 0 {
		return nil, fmt.Errorf("privileged worker: extra processes and opaque Xray full configs are forbidden")
	}
	// This resolver is constructed separately from the JSON configurations.
	// Permit only network resolver schemes, never local-file or Unix transports.
	if addr := in.GetXrayOutboundDnsAddress(); strings.Contains(addr, "://") {
		u, err := url.Parse(addr)
		if err != nil {
			return nil, fmt.Errorf("privileged worker: invalid outbound DNS address")
		}
		switch u.Scheme {
		case "udp", "tcp", "tls", "https", "h3", "quic":
		default:
			return nil, fmt.Errorf("privileged worker: unsupported outbound DNS scheme")
		}
	}
	var box any
	if err := json.Unmarshal([]byte(in.GetCoreConfig()), &box); err != nil {
		return nil, fmt.Errorf("privileged schema v1: invalid sing-box JSON")
	}
	if err := validatePrivilegedNode(box, "box", "sing-box", 0); err != nil {
		return nil, err
	}
	var normalizedXray []byte
	if in.GetNeedXray() || in.GetXrayConfig() != "" {
		var x any
		if err := json.Unmarshal([]byte(in.GetXrayConfig()), &x); err != nil {
			return nil, fmt.Errorf("privileged schema v1: invalid Xray JSON")
		}
		if err := validatePrivilegedNode(x, "xray", "xray", 0); err != nil {
			return nil, err
		}
		normalizedXray, _ = json.Marshal(x)
	}
	// Root never uses the user's cache database, nor the implicit cache.db in
	// its working directory. Fake-IP state remains in memory for this session.
	if exp, ok := box.(map[string]any)["experimental"].(map[string]any); ok {
		if _, exists := exp["cache_file"]; exists {
			exp["cache_file"] = map[string]any{"enabled": false}
		}
	}
	data, err := json.Marshal(box)
	if err != nil {
		return nil, err
	}
	copy := proto.Clone(in).(*gen.LoadConfigReq)
	copy.CoreConfig = To(string(data))
	if normalizedXray != nil {
		copy.XrayConfig = To(string(normalizedXray))
	}
	return copy, nil
}
