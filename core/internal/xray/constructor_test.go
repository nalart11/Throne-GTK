package xray

import (
	"bytes"
	"fmt"
	"log"
	"net"
	"strings"
	"testing"
)

func TestConfigurationErrorsDoNotExposeSecrets(t *testing.T) {
	const secret = "sentinel-password-token-70c0c31f-cb63-4c67-a25f-e80af63e517a"
	for name, config := range map[string]string{
		"syntax":     fmt.Sprintf(`{"password":%q,"outbounds":[`, secret),
		"type":       fmt.Sprintf(`{"outbounds":%q}`, secret),
		"protocol":   fmt.Sprintf(`{"outbounds":[{"protocol":%q}]}`, secret),
		"flow":       fmt.Sprintf(`{"outbounds":[{"protocol":"vless","settings":{"vnext":[{"address":"127.0.0.1","port":443,"users":[{"id":"70c0c31f-cb63-4c67-a25f-e80af63e517a","encryption":"none","flow":%q}]}]}}]}`, secret),
		"encryption": fmt.Sprintf(`{"outbounds":[{"protocol":"vless","settings":{"vnext":[{"address":"127.0.0.1","port":443,"users":[{"id":"70c0c31f-cb63-4c67-a25f-e80af63e517a","encryption":%q}]}]}}]}`, secret),
	} {
		t.Run(name, func(t *testing.T) {
			instance, err := CreateXrayInstance(config)
			if instance != nil {
				_ = instance.Close()
			}
			assertSafeConfigError(t, err, secret)
			assertSafeConfigError(t, CheckXrayConfig(config), secret)

			// Exercise the gate's actual on-demand failure logging path without
			// opening a listener or making a remote connection.
			var logs bytes.Buffer
			old := log.Writer()
			log.SetOutput(&logs)
			defer log.SetOutput(old)
			g := &Gate{config: config, conns: make(map[net.Conn]struct{})}
			client, peer := net.Pipe()
			defer peer.Close()
			g.handle(client, "unused")
			if !strings.Contains(logs.String(), "failed to bring up") || strings.Contains(logs.String(), secret) {
				t.Fatal("missing or unsafe gate failure log")
			}
		})
	}
}

func TestGateParserErrorsDoNotExposeValues(t *testing.T) {
	const secret = "sentinel-secret-password-token"
	for _, config := range []string{
		fmt.Sprintf(`{"inbounds":%q}`, secret),
		fmt.Sprintf(`{"inbounds":[{"port":1,"listen":%q}]}`, secret+":"),
	} {
		_, _, err := remapInbounds(config)
		assertSafeConfigError(t, err, secret)
	}
}

func assertSafeConfigError(t *testing.T, err error, secret string) {
	t.Helper()
	if err == nil {
		t.Fatal("expected configuration error")
	}
	if strings.Contains(err.Error(), secret) || strings.Contains(err.Error(), "70c0c31f-cb63-4c67-a25f-e80af63e517a") {
		t.Fatal("configuration error exposes a supplied credential")
	}
	if !strings.Contains(err.Error(), "xray") {
		t.Fatal("missing safe error context")
	}
}
