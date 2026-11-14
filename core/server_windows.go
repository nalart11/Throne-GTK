package main

import (
	"context"
	"github.com/nalart11/Nolurix/core/gen"
	"github.com/nalart11/Nolurix/core/internal/boxdns"
)

func (s *server) SetSystemDNS(ctx context.Context, in *gen.SetSystemDNSRequest) (*gen.EmptyResp, error) {
	err := boxdns.DnsManagerInstance.SetSystemDNS(nil, *in.Clear)
	if err != nil {
		return nil, err
	}

	return &gen.EmptyResp{}, nil
}
