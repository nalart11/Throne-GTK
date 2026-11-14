package main

import (
	"context"
	"errors"
	"github.com/nalart11/Nolurix/core/gen"
	"github.com/nalart11/Nolurix/core/internal/boxdns"
)

// GetDefaultInterface reports the physical default-route interface tracked by
// the always-on boxdns monitor. The monitor runs on every platform and excludes
// virtual (TUN) and loopback interfaces, so the result is safe to bind a core
// egress to even while throne-tun is up.
func (s *server) GetDefaultInterface(ctx context.Context, in *gen.EmptyReq) (*gen.GetDefaultInterfaceResponse, error) {
	ifc := boxdns.DefaultInterface()
	if ifc == nil {
		return nil, errors.New("no default interface")
	}
	return &gen.GetDefaultInterfaceResponse{
		Name:  To(ifc.Name),
		Index: To(int32(ifc.Index)),
	}, nil
}
