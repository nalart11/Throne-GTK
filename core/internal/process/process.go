package process

import (
	"fmt"
	"os"
	"os/exec"
	"strings"

	"github.com/sagernet/sing/common/atomic"
)

const extraCorePrefix = "Extra Core"

type Process struct {
	path        string
	args        []string
	noOut       bool
	cleanupPath string
	run         running
	stopped     atomic.Bool
}

// running is the minimal handle Process needs to a started child. The normal
// launch (plain exec, or unix setuid) is backed by *exec.Cmd; the elevated
// Windows launch (CreateProcessWithTokenW) is backed by a raw process handle.
type running interface {
	Wait() error
	Kill() error
}

func NewProcess(path string, args []string, noOut bool) *Process {
	return &Process{path: path, args: args, noOut: noOut}
}

// SetCleanupPath registers a filesystem path whose lifetime is bound to the
// process: it is removed (recursively) when the process is stopped, exits, or
// fails to start. It may be the config file itself, or a private directory
// that contains it. Pass "" for nothing to clean up.
func (p *Process) SetCleanupPath(path string) {
	p.cleanupPath = path
}

func (p *Process) Start() error {
	// The Core may run elevated (setuid-root on unix, UAC-elevated on Windows),
	// but the extra process is an arbitrary user-supplied binary that must not
	// inherit those privileges. startChild drops to the unprivileged real user,
	// or refuses to start it at all.
	run, err := startChild(p.path, p.args, p.noOut)
	if err != nil {
		p.cleanup()
		return err
	}
	p.run = run
	p.stopped.Store(false)

	go func() {
		fmt.Println(p.path, ":", "process started, waiting for it to end")
		_ = p.run.Wait()
		if !p.stopped.Load() {
			fmt.Println("Extra process exited unexpectedly")
		}
		p.cleanup()
	}()
	return nil
}

func (p *Process) Stop() {
	p.stopped.Store(true)
	if p.run != nil {
		_ = p.run.Kill()
	}
	p.cleanup()
}

// newCmd builds the exec.Cmd shared by the launch paths that go through Go's
// os/exec: stdout/stderr funnel into the Throne log, and the child environment
// has THRONE* stripped. Platform-specific privilege handling (unix setuid) is
// layered on by startChild before the command is started.
func newCmd(path string, args []string, noOut bool) *exec.Cmd {
	cmd := exec.Command(path, args...)
	cmd.Stdout = &pipeLogger{prefix: extraCorePrefix, noOut: noOut}
	cmd.Stderr = &pipeLogger{prefix: extraCorePrefix, noOut: noOut}
	cmd.Env = childEnv()
	return cmd
}

// startCmd starts cmd and wraps it as a running handle.
func startCmd(cmd *exec.Cmd) (running, error) {
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	return &cmdRunner{cmd: cmd}, nil
}

type cmdRunner struct{ cmd *exec.Cmd }

func (c *cmdRunner) Wait() error { return c.cmd.Wait() }
func (c *cmdRunner) Kill() error { return c.cmd.Process.Kill() }

// cleanup removes the bound path, if any. It is safe to call multiple times and
// from multiple goroutines (RemoveAll on a missing path is a no-op, and unlink
// does not traverse a final symlink, so this cannot be turned into a delete of
// an attacker-chosen target).
func (p *Process) cleanup() {
	if p.cleanupPath != "" {
		_ = os.RemoveAll(p.cleanupPath)
	}
}

// CreateExtraConfig writes the extra process configuration to a fresh file and
// returns (configPath, cleanupPath).
//
// Security model: the Core may be setuid-root / UAC-elevated, and the extra
// process runs as the unprivileged user. The config is written so that the
// unprivileged user cannot turn this into a privileged file operation:
//
//   - createSecureConfigFile creates the file with O_CREATE|O_EXCL (atomic,
//     never follows/clobbers an existing path) and, when elevated, inside a
//     fresh private directory NOT derived from the user-controlled $TMPDIR.
//   - All ownership/permission changes are performed on the open file
//     descriptor (fchown/fchmod), never by re-resolving the path, so a
//     symlink swapped in after creation cannot redirect a privileged chmod/
//     chown onto an attacker-chosen target (the classic TOCTOU).
//
// configPath is what the extra process reads; cleanupPath is what Process must
// remove afterwards (the file, or its private parent directory).
func CreateExtraConfig(content string) (string, string, error) {
	f, cleanupPath, err := createSecureConfigFile()
	if err != nil {
		return "", "", err
	}
	configPath := f.Name()

	fail := func(e error) (string, string, error) {
		_ = f.Close()
		_ = os.RemoveAll(cleanupPath)
		return "", "", e
	}

	if _, err = f.WriteString(content); err != nil {
		return fail(err)
	}
	// The extra process is de-privileged (see applyPrivilegeDrop), so it must
	// still be able to read the config this (possibly elevated) Core wrote.
	// Done on the fd, before Close, so it cannot be hijacked via the path.
	if err = makeConfigReadable(f); err != nil {
		return fail(err)
	}
	if err = f.Close(); err != nil {
		_ = os.RemoveAll(cleanupPath)
		return "", "", err
	}
	return configPath, cleanupPath, nil
}

// childEnv returns the parent environment minus any THRONE-prefixed variables,
// which carry app-internal configuration the external process must not see.
func childEnv() []string {
	parent := os.Environ()
	out := make([]string, 0, len(parent))
	for _, kv := range parent {
		if name, _, ok := strings.Cut(kv, "="); ok && strings.HasPrefix(strings.ToUpper(name), "THRONE") {
			continue
		}
		out = append(out, kv)
	}
	return out
}
