package main

import (
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

func TestAdmissionCapacityAndControlReserve(t *testing.T) {
	a := newHandlerAdmission()
	var releases []func()
	for i := 0; i < 8; i++ {
		r, ok := a.acquire("Test")
		if !ok {
			t.Fatal("normal capacity too small")
		}
		releases = append(releases, r)
	}
	if _, ok := a.acquire("Start"); ok {
		t.Fatal("normal overflow admitted")
	}
	for _, method := range []string{"Stop", "StopTest"} {
		r, ok := a.acquire(method)
		if !ok {
			t.Fatal("control reserve unavailable")
		}
		releases = append(releases, r)
	}
	if _, ok := a.acquire("Stop"); ok {
		t.Fatal("control overflow admitted")
	}
	for _, r := range releases {
		r()
	}
	if r, ok := a.acquire("Test"); !ok {
		t.Fatal("capacity not released")
	} else {
		r()
	}
}

func TestAdmissionRejectsBeforePayloadRead(t *testing.T) {
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	go func() { b.Write(request("Test", nil)[:10]) }() // ID + length + method only
	_, _, _, err := readAdmittedRequestFrame(a, time.Second, func(method string) bool {
		if method != "Test" {
			t.Errorf("wrong method: %q", method)
		}
		return false
	})
	if err == nil {
		t.Fatal("overflow accepted")
	}
	if ne, ok := err.(net.Error); ok && ne.Timeout() {
		t.Fatal("read payload before admission")
	}
}

func TestBoundedCleanup(t *testing.T) {
	blocked := make(chan struct{})
	finished := make(chan struct{}, 1)
	done := make(chan struct{})
	go func() {
		boundedCleanup(20*time.Millisecond, func() { <-blocked }, func() { finished <- struct{}{} }, func() { panic("private") })
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("cleanup exceeded bound")
	}
	select {
	case <-finished:
	default:
		t.Fatal("blocked cleanup prevented other task")
	}
	close(blocked)
}

func request(method string, payload []byte) []byte {
	b := new(bytes.Buffer)
	binary.Write(b, binary.LittleEndian, uint32(42))
	binary.Write(b, binary.LittleEndian, uint16(len(method)))
	b.WriteString(method)
	binary.Write(b, binary.LittleEndian, uint32(len(payload)))
	b.Write(payload)
	return b.Bytes()
}

func TestReadFrames(t *testing.T) {
	oversized := request("Stop", nil)
	binary.LittleEndian.PutUint32(oversized[len(oversized)-4:], maxFrameBytes+1)
	for _, tc := range []struct {
		name  string
		data  []byte
		valid bool
	}{
		{"valid", request("Stop", []byte("abc")), true},
		{"empty method", request("", nil), false},
		{"long method", request(string(make([]byte, 257)), nil), false},
		{"oversized payload", oversized, false},
		{"truncated header", []byte{1, 2}, false},
		{"truncated payload", request("Stop", []byte("abc"))[:15], false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a, b := net.Pipe()
			defer a.Close()
			defer b.Close()
			go func() {
				b.Write(tc.data)
				if !tc.valid {
					b.Close()
				}
			}()
			id, method, payload, err := readRequestFrame(a, time.Second)
			if tc.valid {
				if err != nil || id != 42 || method != "Stop" || string(payload) != "abc" {
					t.Fatalf("%d %q %q %v", id, method, payload, err)
				}
			} else if err == nil {
				t.Fatal("accepted malformed frame")
			}
		})
	}
}

func TestReadDeadlineAndIdle(t *testing.T) {
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	done := make(chan error, 1)
	go func() { _, _, _, err := readRequestFrame(a, 20*time.Millisecond); done <- err }()
	select {
	case err := <-done:
		t.Fatalf("idle read returned: %v", err)
	case <-time.After(50 * time.Millisecond):
	}
	b.Write([]byte{1})
	select {
	case err := <-done:
		var ne net.Error
		if !errors.As(err, &ne) || !ne.Timeout() {
			t.Fatalf("expected timeout: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("partial frame blocked")
	}
}

type shortConn struct {
	net.Conn
	bytes.Buffer
	zero   bool
	fail   bool
	closed bool
}

func (c *shortConn) Read(p []byte) (int, error) { return c.Buffer.Read(p) }
func (c *shortConn) Write(p []byte) (int, error) {
	if c.zero {
		return 0, nil
	}
	if c.fail {
		return 1, io.ErrUnexpectedEOF
	}
	if len(p) > 2 {
		p = p[:2]
	}
	return c.Buffer.Write(p)
}
func (c *shortConn) SetWriteDeadline(time.Time) error { return nil }
func (c *shortConn) Close() error                     { c.closed = true; return nil }

func TestResponseWrites(t *testing.T) {
	c := new(shortConn)
	if err := writeResponseFrame(c, 42, 1, []byte("abc"), time.Second); err != nil {
		t.Fatal(err)
	}
	expected := []byte{42, 0, 0, 0, 1, 3, 0, 0, 0, 'a', 'b', 'c'}
	if !bytes.Equal(c.Bytes(), expected) {
		t.Fatalf("wrong frame: %v", c.Bytes())
	}
	for _, c := range []*shortConn{{zero: true}, {fail: true}} {
		if err := writeResponseFrame(c, 1, 0, nil, time.Second); err == nil || !c.closed {
			t.Fatal("write failure did not close connection")
		}
	}
	c = new(shortConn)
	if err := writeResponseFrame(c, 1, 0, make([]byte, maxFrameBytes+1), time.Second); err == nil || !c.closed || c.Len() != 0 {
		t.Fatal("oversized response accepted")
	}
}

func TestWriteDeadlineClosesReader(t *testing.T) {
	a, b := net.Pipe()
	defer b.Close()
	done := make(chan error, 1)
	go func() { var p [1]byte; _, err := a.Read(p[:]); done <- err }()
	if err := writeResponseFrame(a, 1, 0, nil, 20*time.Millisecond); err == nil {
		t.Fatal("expected write timeout")
	}
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("reader not failed")
		}
	case <-time.After(time.Second):
		t.Fatal("reader not woken")
	}
}
