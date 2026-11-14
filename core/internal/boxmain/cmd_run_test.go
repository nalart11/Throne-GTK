package boxmain

import (
	"fmt"
	"strings"
	"testing"
)

func TestParseConfigErrorsDoNotExposeCredentials(t *testing.T) {
	const password = "sentinel-password-secret"
	const uuid = "70c0c31f-cb63-4c67-a25f-e80af63e517a"
	const token = "sentinel-token-secret"
	for _, tc := range []struct {
		name, config, diagnostic string
	}{
		{"syntax", fmt.Sprintf(`{"password":%q,"uuid":%q,"token":%q,`, password, uuid, token), "invalid JSON syntax at byte offset"},
		{"unknown field", fmt.Sprintf(`{%q:{"password":%q,"uuid":%q,"token":%q}}`, token, password, uuid, token), "invalid configuration"},
		{"unknown outbound type", fmt.Sprintf(`{"outbounds":[{"type":%q,"password":%q,"uuid":%q,"token":%q}]}`, token, password, uuid, token), "invalid configuration"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			options, err := parseConfig(newBoxContext(), []byte(tc.config))
			if err == nil || options != nil {
				t.Fatal("expected failed config parse")
			}
			for _, secret := range []string{password, uuid, token, tc.config} {
				if strings.Contains(err.Error(), secret) {
					t.Fatal("config error exposes sensitive input")
				}
			}
			if !strings.Contains(err.Error(), "decode config: "+tc.diagnostic) {
				t.Fatalf("missing safe diagnostic: %s", err)
			}
		})
	}
}
