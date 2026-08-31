package main

import (
	"ThroneGtkCore/gen"
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"log"
	"net"
	runtimeDebug "runtime/debug"
	"sync"

	"google.golang.org/protobuf/proto"
)

var globalServer = &server{}

// runDispatch reads request frames and dispatches each to its own goroutine.
// Request:  [uint32 reqId][uint16 methodLen][method][uint32 payloadLen][payload]
// Response: [uint32 reqId][uint8 status][uint32 dataLen][data]   (little-endian)
func runDispatch(conn net.Conn) {
	defer func() {
		conn.Close()
		log.Fatal("IPC connection dropped, exiting")
	}()

	var writeMu sync.Mutex

	writeResponse := func(reqId uint32, status uint8, data []byte) {
		var header [9]byte
		binary.LittleEndian.PutUint32(header[0:], reqId)
		header[4] = status
		binary.LittleEndian.PutUint32(header[5:], uint32(len(data)))

		writeMu.Lock()
		defer writeMu.Unlock()
		if _, err := conn.Write(header[:]); err != nil {
			return
		}
		if len(data) > 0 {
			if _, err := conn.Write(data); err != nil {
				return
			}
		}
	}

	for {
		// Read reqId (uint32 LE)
		var reqId uint32
		if err := binary.Read(conn, binary.LittleEndian, &reqId); err != nil {
			return
		}

		// Read method name length (uint16 LE)
		var methodLen uint16
		if err := binary.Read(conn, binary.LittleEndian, &methodLen); err != nil {
			return
		}

		// Read method name
		methodBytes := make([]byte, methodLen)
		if _, err := io.ReadFull(conn, methodBytes); err != nil {
			return
		}

		// Read payload length (uint32 LE)
		var payloadLen uint32
		if err := binary.Read(conn, binary.LittleEndian, &payloadLen); err != nil {
			return
		}

		// Read payload
		payload := make([]byte, payloadLen)
		if _, err := io.ReadFull(conn, payload); err != nil {
			return
		}

		// Dispatch concurrently so long-running calls don't block the reader.
		go func(id uint32, method string, pl []byte) {
			// main()'s recover only covers the main goroutine, so without this
			// one bad request takes the whole core down.
			defer func() {
				if r := recover(); r != nil {
					log.Printf("panic in %s: %v\n%s", method, r, runtimeDebug.Stack())
					writeResponse(id, 1, []byte(fmt.Sprintf("core panic in %s: %v", method, r)))
				}
			}()
			respData, dispatchErr := dispatch(method, pl)
			if dispatchErr != nil {
				writeResponse(id, 1, []byte(dispatchErr.Error()))
			} else {
				writeResponse(id, 0, respData)
			}
		}(reqId, string(methodBytes), payload)
	}
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
	h, found := handlers[methodName]
	if !found {
		return nil, fmt.Errorf("unknown method: %s", methodName)
	}
	return h(context.Background(), payload)
}

// The wire path goes through the table above, not gRPC; keep the interface honest.
var _ gen.LibcoreServiceServer = globalServer
