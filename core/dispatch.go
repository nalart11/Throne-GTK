package main

import (
	"ThroneGtkCore/gen"
	"context"
	"fmt"
	"log"
	"net"
	runtimeDebug "runtime/debug"
	"sync"
	"time"

	"google.golang.org/protobuf/proto"
)

var globalServer = &server{}

// runDispatch reads request frames and dispatches each to its own goroutine.
// Request:  [uint32 reqId][uint16 methodLen][method][uint32 payloadLen][payload]
// Response: [uint32 reqId][uint8 status][uint32 dataLen][data]   (little-endian)
func runDispatch(conn net.Conn) {
	ctx, cancel := context.WithCancel(context.Background())
	defer func() {
		cancel()
		conn.Close()
		boundedCleanup(5*time.Second,
			func() { _, _ = globalServer.StopTest(context.Background(), &gen.EmptyReq{}) },
			func() { _, _ = globalServer.Stop(context.Background(), &gen.EmptyReq{}) },
		)
		log.Print("IPC connection dropped, exiting")
	}()

	var writeMu sync.Mutex
	admission := newHandlerAdmission()

	writeResponse := func(reqId uint32, status uint8, data []byte) {
		writeMu.Lock()
		defer writeMu.Unlock()
		if err := writeResponseFrame(conn, reqId, status, data, frameTimeout); err != nil {
			log.Printf("IPC response write: %v", err)
		}
	}

	for {
		var release func()
		reqId, method, payload, err := readAdmittedRequestFrame(conn, frameTimeout, func(method string) bool {
			var ok bool
			release, ok = admission.acquire(method)
			return ok
		})
		if err != nil {
			if release != nil {
				release()
			}
			return
		}

		// Dispatch concurrently so long-running calls don't block the reader.
		go func(id uint32, method string, pl []byte, release func()) {
			defer release()
			// main()'s recover only covers the main goroutine, so without this
			// one bad request takes the whole core down.
			defer func() {
				if r := recover(); r != nil {
					log.Printf("panic in %s\n%s", safeDispatchMethod(method), runtimeDebug.Stack())
					writeResponse(id, 1, []byte("core handler panic"))
				}
			}()
			respData, dispatchErr := dispatchContext(ctx, method, pl)
			if dispatchErr != nil {
				writeResponse(id, 1, []byte(dispatchErr.Error()))
			} else {
				writeResponse(id, 0, respData)
			}
		}(reqId, method, payload, release)
	}
}

func safeDispatchMethod(method string) string {
	if _, ok := handlers[method]; ok {
		return method
	}
	return "unknown"
}

type handlerFn func(context.Context, []byte) ([]byte, error)

// handle adapts a typed server method to the wire: unmarshal, call, marshal.
// PReq pins Req to the pointer type the generated code implements proto.Message on.
func handle[Req any, PReq interface {
	*Req
	proto.Message
}, Resp proto.Message](fn func(context.Context, PReq) (Resp, error)) handlerFn {
	return func(ctx context.Context, payload []byte) ([]byte, error) {
		req := PReq(new(Req))
		if err := proto.Unmarshal(payload, req); err != nil {
			return nil, err
		}
		resp, err := fn(ctx, req)
		if err != nil {
			return nil, err
		}
		return proto.Marshal(resp)
	}
}

// The whole RPC surface. Request types are inferred from each method signature.
var handlers = map[string]handlerFn{
	"Start":               handle(globalServer.Start),
	"Stop":                handle(globalServer.Stop),
	"CheckConfig":         handle(globalServer.CheckConfig),
	"Test":                handle(globalServer.Test),
	"StopTest":            handle(globalServer.StopTest),
	"QueryURLTest":        handle(globalServer.QueryURLTest),
	"IPTest":              handle(globalServer.IPTest),
	"QueryIPTest":         handle(globalServer.QueryIPTest),
	"QueryStats":          handle(globalServer.QueryStats),
	"QueryConnections":    handle(globalServer.QueryConnections),
	"QueryAutoSelectors":  handle(globalServer.QueryAutoSelectors),
	"AutoSelectorAction":  handle(globalServer.AutoSelectorAction),
	"IsPrivileged":        handle(globalServer.IsPrivileged),
	"SetSystemDNS":        handle(globalServer.SetSystemDNS),
	"GetDefaultInterface": handle(globalServer.GetDefaultInterface),
	"SpeedTest":           handle(globalServer.SpeedTest),
	"QuerySpeedTest":      handle(globalServer.QuerySpeedTest),
	"QueryCountryTest":    handle(globalServer.QueryCountryTest),
	"GenWgKeyPair":        handle(globalServer.GenWgKeyPair),
}

func dispatch(methodName string, payload []byte) ([]byte, error) {
	return dispatchContext(context.Background(), methodName, payload)
}

func dispatchContext(ctx context.Context, methodName string, payload []byte) ([]byte, error) {
	h, found := handlers[methodName]
	if !found {
		return nil, fmt.Errorf("unknown method: %s", methodName)
	}
	return h(ctx, payload)
}

// The wire path goes through the table above, not gRPC; keep the interface honest.
var _ gen.LibcoreServiceServer = globalServer
