package helpertrust

import (
	"bytes"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"testing"
)

type shortWriter struct{ bytes.Buffer }

func (w *shortWriter) Write(p []byte) (int, error) {
	if len(p) > 2 {
		p = p[:2]
	}
	return w.Buffer.Write(p)
}

type zeroWriter struct{}

func (zeroWriter) Write([]byte) (int, error) { return 0, nil }

type errorWriter struct{}

func (errorWriter) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }

func TestWriteFull(t *testing.T) {
	w := new(shortWriter)
	if err := WriteFull(w, []byte("complete frame")); err != nil || w.String() != "complete frame" {
		t.Fatalf("payload=%q error=%v", w.String(), err)
	}
	if !errors.Is(WriteFull(zeroWriter{}, []byte("x")), io.ErrShortWrite) {
		t.Fatal("zero write accepted")
	}
	if !errors.Is(WriteFull(errorWriter{}, []byte("x")), io.ErrClosedPipe) {
		t.Fatal("write error lost")
	}
}

func TestSocketRecovery(t *testing.T) {
	p := filepath.Join(t.TempDir(), "helper.sock")
	if err := RemoveStaleSocket(p); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte("keep"), 0600); err != nil {
		t.Fatal(err)
	}
	if RemoveStaleSocket(p) == nil {
		t.Fatal("accepted regular file")
	}
	if err := os.Remove(p); err != nil {
		t.Fatal(err)
	}
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: p, Net: "unix"})
	if err != nil {
		t.Fatal(err)
	}
	listener.SetUnlinkOnClose(false)
	defer listener.Close()
	if RemoveStaleSocket(p) == nil {
		t.Fatal("removed live/foreign socket")
	}
	if _, err := os.Lstat(p); err != nil {
		t.Fatal("live endpoint removed:", err)
	}
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	if os.Getuid() != 0 {
		if RemoveStaleSocket(p) == nil {
			t.Fatal("removed foreign socket")
		}
		t.Log("root-owned stale recovery unavailable without root; foreign socket rejection verified")
		return
	}
	if err := RemoveStaleSocket(p); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(p); !os.IsNotExist(err) {
		t.Fatalf("stale endpoint remains: %v", err)
	}
}
