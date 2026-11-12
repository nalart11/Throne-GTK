package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"time"
)

const (
	maxFrameBytes  = 64 * 1024 * 1024
	maxMethodBytes = 256
	frameTimeout   = 15 * time.Second
)

// Idle waiting is unbounded; the first byte starts a deadline for the whole frame.
func readRequestFrame(conn net.Conn, timeout time.Duration) (id uint32, method string, payload []byte, err error) {
	return readAdmittedRequestFrame(conn, timeout, nil)
}

func readAdmittedRequestFrame(conn net.Conn, timeout time.Duration, admit func(string) bool) (id uint32, method string, payload []byte, err error) {
	var header [6]byte
	if _, err = io.ReadFull(conn, header[:1]); err != nil {
		return
	}
	if err = conn.SetReadDeadline(time.Now().Add(timeout)); err != nil {
		return
	}
	defer func() {
		if e := conn.SetReadDeadline(time.Time{}); err == nil {
			err = e
		}
	}()
	if _, err = io.ReadFull(conn, header[1:]); err != nil {
		return
	}
	id = binary.LittleEndian.Uint32(header[:4])
	n := binary.LittleEndian.Uint16(header[4:])
	if n == 0 || n > maxMethodBytes {
		err = fmt.Errorf("invalid method length: %d", n)
		return
	}
	name := make([]byte, int(n))
	if _, err = io.ReadFull(conn, name); err != nil {
		return
	}
	method = string(name)
	if admit != nil && !admit(method) {
		err = fmt.Errorf("handler capacity exhausted")
		return
	}
	var size [4]byte
	if _, err = io.ReadFull(conn, size[:]); err != nil {
		return
	}
	length := binary.LittleEndian.Uint32(size[:])
	if length > maxFrameBytes {
		err = fmt.Errorf("payload too large: %d", length)
		return
	}
	payload = make([]byte, int(length))
	_, err = io.ReadFull(conn, payload)
	method = string(name)
	return
}

// Separate control slots ensure normal calls cannot starve Stop/StopTest.
type handlerAdmission struct {
	normal  chan struct{}
	control chan struct{}
}

func newHandlerAdmission() *handlerAdmission {
	return &handlerAdmission{make(chan struct{}, 8), make(chan struct{}, 2)}
}

func (a *handlerAdmission) acquire(method string) (func(), bool) {
	pool := a.normal
	if method == "Stop" || method == "StopTest" {
		pool = a.control
	}
	select {
	case pool <- struct{}{}:
		return func() { <-pool }, true
	default:
		return nil, false
	}
}

// Each cleanup runs independently: a blocked Stop must not prevent StopTest.
func boundedCleanup(timeout time.Duration, tasks ...func()) {
	done := make(chan struct{}, len(tasks))
	for _, task := range tasks {
		go func() {
			defer func() { _ = recover(); done <- struct{}{} }()
			task()
		}()
	}
	timer := time.NewTimer(timeout)
	defer timer.Stop()
	for range tasks {
		select {
		case <-done:
		case <-timer.C:
			return
		}
	}
}

func writeFull(w io.Writer, data []byte) error {
	for len(data) > 0 {
		n, err := w.Write(data)
		if n < 0 || n > len(data) {
			return io.ErrShortWrite
		}
		if err != nil {
			return err
		}
		if n == 0 {
			return io.ErrNoProgress
		}
		data = data[n:]
	}
	return nil
}

// Caller serializes frames. Close on every error to wake the reader and prevent
// subsequent responses from following a partial frame.
func writeResponseFrame(conn net.Conn, id uint32, status uint8, data []byte, timeout time.Duration) (err error) {
	defer func() {
		if err != nil {
			conn.Close()
		}
	}()
	if len(data) > maxFrameBytes {
		return fmt.Errorf("response too large: %d", len(data))
	}
	if err = conn.SetWriteDeadline(time.Now().Add(timeout)); err != nil {
		return err
	}
	defer func() {
		if e := conn.SetWriteDeadline(time.Time{}); err == nil {
			err = e
		}
	}()
	var header [9]byte
	binary.LittleEndian.PutUint32(header[:4], id)
	header[4] = status
	binary.LittleEndian.PutUint32(header[5:], uint32(len(data)))
	if err = writeFull(conn, header[:]); err != nil {
		return err
	}
	return writeFull(conn, data)
}
