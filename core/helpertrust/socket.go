package helpertrust

import (
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"syscall"
	"time"
)

// RemoveStaleSocket requires the caller to hold the singleton lock and protect
// the containing directory. Only ECONNREFUSED proves a stale endpoint; timeouts,
// permission failures and other ambiguous failures never authorize removal.
func RemoveStaleSocket(path string) error {
	before, err := os.Lstat(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	st, ok := before.Sys().(*syscall.Stat_t)
	if !ok || st.Uid != 0 || before.Mode()&os.ModeSocket == 0 {
		return fmt.Errorf("refusing foreign/non-socket endpoint %s", path)
	}
	c, err := net.DialTimeout("unix", path, time.Second)
	if err == nil {
		c.Close()
		return fmt.Errorf("helper endpoint has an active listener")
	}
	if !errors.Is(err, syscall.ECONNREFUSED) {
		return fmt.Errorf("cannot prove helper socket stale: %w", err)
	}
	after, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if !os.SameFile(before, after) {
		return fmt.Errorf("helper socket inode changed")
	}
	return os.Remove(path)
}

func WriteFull(w io.Writer, data []byte) error {
	for len(data) > 0 {
		n, err := w.Write(data)
		if n < 0 || n > len(data) {
			return io.ErrShortWrite
		}
		data = data[n:]
		if err != nil {
			return err
		}
		if n == 0 {
			return io.ErrShortWrite
		}
	}
	return nil
}
