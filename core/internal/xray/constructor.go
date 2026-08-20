package xray

import (
	"bytes"
	"github.com/xtls/xray-core/core"
	"github.com/xtls/xray-core/infra/conf/serial"
)

func CreateXrayInstance(config string) (*core.Instance, error) {
	r := bytes.NewReader([]byte(config))
	conf, err := serial.DecodeJSONConfig(r)
	if err != nil {
		return nil, err
	}

	b, err := conf.Build()
	if err != nil {
		return nil, err
	}

	server, err := core.New(b)
	if err != nil {
		return nil, err
	}

	return server, nil
}

// CheckXrayConfig validates an Xray JSON config without creating a running
// instance. Decoding plus conf.Build() parses and validates the protocol
// settings (UUIDs, flow, encryption, stream/TLS/reality settings, etc.), which
// is everything that determines whether a profile is well-formed. It
// deliberately stops short of core.New: that would instantiate a full set of
// handlers just to throw them away, and validation runs concurrently (bulk
// "remove invalid configs") while a live Xray instance is up.
func CheckXrayConfig(config string) error {
	r := bytes.NewReader([]byte(config))
	conf, err := serial.DecodeJSONConfig(r)
	if err != nil {
		return err
	}
	_, err = conf.Build()
	return err
}
