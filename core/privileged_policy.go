package main

import (
	"context"
	"errors"
	"sync/atomic"
)

// Enabled only by the dedicated helper bootstrap, never inferred from uid or
// client configuration. Ordinary core processes retain their existing surface.
var privilegedWorker atomic.Bool

func enablePrivilegedWorkerPolicy() { privilegedWorker.Store(true) }

// AuthorizePrivilegedWorkerMethod is also available to dispatchers before they
// decode requests. Keep unknown methods denied when extending the RPC surface.
func AuthorizePrivilegedWorkerMethod(method string) error {
	if !privilegedWorker.Load() {
		return nil
	}
	switch method {
	case "Start", "Stop", "IsPrivileged", "GetDefaultInterface", "QueryStats", "QueryConnections", "QueryAutoSelectors", "AutoSelectorAction":
		return nil
	default:
		return errors.New("privileged worker: method not authorized")
	}
}

func init() {
	// Cover every existing dispatch entry (including SetSystemDNS) without
	// changing dispatch.go. Authorization precedes protobuf decoding and any
	// handler side effect; check the flag at invocation, not registration.
	for method, handler := range handlers {
		handlers[method] = func(ctx context.Context, payload []byte) ([]byte, error) {
			if err := AuthorizePrivilegedWorkerMethod(method); err != nil {
				return nil, err
			}
			return handler(ctx, payload)
		}
	}
}
