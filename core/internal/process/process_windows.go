//go:build windows

package process

import (
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

// startChild launches the extra process de-elevated to the unprivileged
// interactive user when the Core runs elevated, else as-is.
//
// Windows has no setuid: a UAC-elevated parent spawns children with its own
// elevated admin token. We obtain a non-elevated primary token for the real
// user and launch the child with it.
//
// We deliberately do NOT use Go's exec.Cmd + SysProcAttr.Token: that routes
// through CreateProcessAsUser, which requires SeAssignPrimaryTokenPrivilege — a
// privilege a UAC-elevated administrator does not hold (only SYSTEM does), so it
// fails with ERROR_PRIVILEGE_NOT_HELD ("a required privilege is not held by the
// client"). We instead call CreateProcessWithTokenW, which needs only
// SeImpersonatePrivilege, held by elevated admins.
// See https://github.com/throneproj/Throne/issues/1482.
func startChild(path string, args []string, noOut bool) (running, error) {
	self, err := selfToken()
	if err != nil {
		return nil, fmt.Errorf("cannot open process token: %w", err)
	}
	defer self.Close()

	if !self.IsElevated() {
		return startCmd(newCmd(path, args, noOut)) // not elevated, run as-is
	}

	tok, err := unprivilegedToken(self)
	if err != nil {
		return nil, fmt.Errorf("refusing to start extra process: cannot obtain an unprivileged user token: %w", err)
	}
	defer tok.Close()

	return startWithToken(path, args, noOut, tok)
}

var procCreateProcessWithTokenW = windows.NewLazySystemDLL("advapi32.dll").NewProc("CreateProcessWithTokenW")

// startWithToken launches path+args under tok via CreateProcessWithTokenW, with
// stdout/stderr funnelled into the Throne log and no console window.
//
// CreateProcessWithTokenW does not inherit arbitrary handles, but for a 64-bit
// child the secondary-logon service still duplicates the three std handles into
// it, so redirected output works without leaking any other handle to the
// de-privileged child.
func startWithToken(path string, args []string, noOut bool, tok windows.Token) (running, error) {
	exe, err := exec.LookPath(path)
	if err != nil {
		return nil, err
	}

	// stdout/stderr: the child writes the (inheritable) write ends; we read the
	// read ends. stdin is the null device so the child has a valid handle.
	outR, outW, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	errR, errW, err := os.Pipe()
	if err != nil {
		closeAll(outR, outW)
		return nil, err
	}
	nul, err := os.OpenFile("NUL", os.O_RDONLY, 0)
	if err != nil {
		closeAll(outR, outW, errR, errW)
		return nil, err
	}
	for _, f := range []*os.File{outW, errW, nul} {
		if err = makeInheritable(f); err != nil {
			closeAll(outR, outW, errR, errW, nul)
			return nil, err
		}
	}

	si := &windows.StartupInfo{}
	si.Cb = uint32(unsafe.Sizeof(*si))
	si.Flags = windows.STARTF_USESTDHANDLES | windows.STARTF_USESHOWWINDOW
	si.ShowWindow = windows.SW_HIDE
	si.StdInput = windows.Handle(nul.Fd())
	si.StdOutput = windows.Handle(outW.Fd())
	si.StdErr = windows.Handle(errW.Fd())

	appName, err := windows.UTF16PtrFromString(exe)
	if err != nil {
		closeAll(outR, outW, errR, errW, nul)
		return nil, err
	}
	cmdLine, err := windows.UTF16PtrFromString(windows.ComposeCommandLine(append([]string{exe}, args...)))
	if err != nil {
		closeAll(outR, outW, errR, errW, nul)
		return nil, err
	}
	envBlock, err := makeEnvBlock(childEnv())
	if err != nil {
		closeAll(outR, outW, errR, errW, nul)
		return nil, err
	}

	// dwLogonFlags 0: don't load the user profile (the old launch didn't either).
	const createFlags = windows.CREATE_UNICODE_ENVIRONMENT | windows.CREATE_NO_WINDOW
	var pi windows.ProcessInformation
	r1, _, e1 := procCreateProcessWithTokenW.Call(
		uintptr(tok),
		0,
		uintptr(unsafe.Pointer(appName)),
		uintptr(unsafe.Pointer(cmdLine)),
		uintptr(createFlags),
		uintptr(unsafe.Pointer(&envBlock[0])),
		0,
		uintptr(unsafe.Pointer(si)),
		uintptr(unsafe.Pointer(&pi)),
	)
	runtime.KeepAlive(si)
	runtime.KeepAlive(appName)
	runtime.KeepAlive(cmdLine)
	runtime.KeepAlive(envBlock)
	// Our copies of the child's ends are no longer needed; closing the write
	// ends is what lets the read ends see EOF when the child exits.
	closeAll(outW, errW, nul)
	if r1 == 0 {
		closeAll(outR, errR)
		return nil, fmt.Errorf("CreateProcessWithTokenW %s: %w", exe, e1)
	}
	_ = windows.CloseHandle(pi.Thread)

	done := make(chan struct{}, 2)
	pump := func(r *os.File) {
		_, _ = io.Copy(&pipeLogger{prefix: extraCorePrefix, noOut: noOut}, r)
		_ = r.Close()
		done <- struct{}{}
	}
	go pump(outR)
	go pump(errR)

	return &tokenRunner{hProcess: pi.Process, done: done}, nil
}

// tokenRunner is the running handle for a child launched via
// CreateProcessWithTokenW: it owns the process handle and waits for the two
// stdout/stderr pumps (signalled on done) to drain before reporting exit.
type tokenRunner struct {
	mu       sync.Mutex
	hProcess windows.Handle
	done     chan struct{}
}

func (t *tokenRunner) Wait() error {
	t.mu.Lock()
	h := t.hProcess
	t.mu.Unlock()
	if h == 0 {
		return nil
	}
	_, err := windows.WaitForSingleObject(h, windows.INFINITE)
	<-t.done // stdout drained
	<-t.done // stderr drained
	t.mu.Lock()
	if t.hProcess != 0 {
		_ = windows.CloseHandle(t.hProcess)
		t.hProcess = 0
	}
	t.mu.Unlock()
	return err
}

// Kill terminates the child. It is a no-op once Wait has reaped the process, so
// the handle that Wait closes is never used after it is closed.
func (t *tokenRunner) Kill() error {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.hProcess == 0 {
		return nil
	}
	return windows.TerminateProcess(t.hProcess, 1)
}

// makeInheritable marks f's handle inheritable so it can be handed to the child.
func makeInheritable(f *os.File) error {
	return windows.SetHandleInformation(windows.Handle(f.Fd()),
		windows.HANDLE_FLAG_INHERIT, windows.HANDLE_FLAG_INHERIT)
}

func closeAll(files ...*os.File) {
	for _, f := range files {
		_ = f.Close()
	}
}

// makeEnvBlock builds a CREATE_UNICODE_ENVIRONMENT block: each "k=v" UTF-16
// encoded and NUL-terminated, the whole list terminated by one more NUL.
func makeEnvBlock(env []string) ([]uint16, error) {
	var block []uint16
	for _, e := range env {
		if e == "" {
			continue
		}
		u, err := windows.UTF16FromString(e)
		if err != nil {
			return nil, err
		}
		block = append(block, u...) // u already ends in NUL
	}
	block = append(block, 0) // terminate the block
	if len(block) == 1 {
		block = append(block, 0) // an empty environment still needs a double NUL
	}
	return block, nil
}

func selfToken() (windows.Token, error) {
	var tok windows.Token
	err := windows.OpenProcessToken(windows.CurrentProcess(),
		windows.TOKEN_QUERY|windows.TOKEN_DUPLICATE, &tok)
	return tok, err
}

// unprivilegedToken returns a primary token for the real, non-elevated user,
// trying in order: the UAC linked token of the same user (the common
// elevated-admin case), the active console session user, then the interactive
// shell (explorer.exe) user (covers a SYSTEM service).
func unprivilegedToken(self windows.Token) (windows.Token, error) {
	if t, err := linkedToken(self); err == nil {
		return t, nil
	}
	if t, err := consoleSessionToken(); err == nil {
		return t, nil
	}
	if t, err := shellToken(); err == nil {
		return t, nil
	}
	return 0, errors.New("no linked, console-session, or shell token available")
}

func linkedToken(self windows.Token) (windows.Token, error) {
	linked, err := self.GetLinkedToken()
	if err != nil {
		return 0, err
	}
	defer linked.Close()
	if linked.IsElevated() {
		return 0, errors.New("linked token is still elevated")
	}
	return primaryToken(linked)
}

func consoleSessionToken() (windows.Token, error) {
	session := windows.WTSGetActiveConsoleSessionId()
	if session == 0xFFFFFFFF {
		return 0, errors.New("no active console session")
	}
	var tok windows.Token
	if err := windows.WTSQueryUserToken(session, &tok); err != nil {
		return 0, err
	}
	defer tok.Close()
	return primaryToken(tok)
}

func shellToken() (windows.Token, error) {
	pid, err := findProcess("explorer.exe")
	if err != nil {
		return 0, err
	}
	proc, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		return 0, err
	}
	defer windows.CloseHandle(proc)

	var tok windows.Token
	if err := windows.OpenProcessToken(proc, windows.TOKEN_DUPLICATE|windows.TOKEN_QUERY, &tok); err != nil {
		return 0, err
	}
	defer tok.Close()
	return primaryToken(tok)
}

func primaryToken(src windows.Token) (windows.Token, error) {
	var dup windows.Token
	err := windows.DuplicateTokenEx(
		src,
		windows.TOKEN_ASSIGN_PRIMARY|windows.TOKEN_DUPLICATE|windows.TOKEN_QUERY|
			windows.TOKEN_ADJUST_DEFAULT|windows.TOKEN_ADJUST_SESSIONID,
		nil,
		windows.SecurityImpersonation,
		windows.TokenPrimary,
		&dup,
	)
	if err != nil {
		return 0, err
	}
	return dup, nil
}

func findProcess(name string) (uint32, error) {
	snap, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return 0, err
	}
	defer windows.CloseHandle(snap)

	var entry windows.ProcessEntry32
	entry.Size = uint32(unsafe.Sizeof(entry))
	for err = windows.Process32First(snap, &entry); err == nil; err = windows.Process32Next(snap, &entry) {
		if strings.EqualFold(windows.UTF16ToString(entry.ExeFile[:]), name) {
			return entry.ProcessID, nil
		}
	}
	return 0, fmt.Errorf("process %q not found", name)
}

// createSecureConfigFile creates the extra-process config file. On Windows
// %TEMP% is a per-user directory and creating a symlink needs a privilege
// unprivileged users lack, so an ordinary O_CREATE|O_EXCL temp file (the
// default of os.CreateTemp) already creates a clean, un-hijackable file.
func createSecureConfigFile() (*os.File, string, error) {
	f, err := os.CreateTemp("", "throne-extra-*.conf")
	if err != nil {
		return nil, "", err
	}
	return f, f.Name(), nil
}

// makeConfigReadable best-effort grants the local Users group read access so a
// de-privileged child can read the config. To avoid a path swap between
// creation and the ACL change, it reopens the file WITHOUT following a reparse
// point and verifies (by volume + file id) that it is the very object we
// created before touching the DACL through that handle. A failure is
// non-fatal: with the linked-token path the child is the same user at lower
// integrity and can already read it.
func makeConfigReadable(f *os.File) error {
	usersSid, err := windows.CreateWellKnownSid(windows.WinBuiltinUsersSid)
	if err != nil {
		return nil
	}
	h, err := reopenSameObject(f)
	if err != nil {
		return nil
	}
	defer windows.CloseHandle(h)

	sd, err := windows.GetSecurityInfo(h, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		return nil
	}
	dacl, _, err := sd.DACL()
	if err != nil {
		return nil
	}
	entries := []windows.EXPLICIT_ACCESS{{
		AccessPermissions: windows.GENERIC_READ,
		AccessMode:        windows.GRANT_ACCESS,
		Inheritance:       windows.NO_INHERITANCE,
		Trustee: windows.TRUSTEE{
			TrusteeForm:  windows.TRUSTEE_IS_SID,
			TrusteeType:  windows.TRUSTEE_IS_GROUP,
			TrusteeValue: windows.TrusteeValueFromSID(usersSid),
		},
	}}
	merged, err := windows.ACLFromEntries(entries, dacl)
	if err != nil {
		return nil
	}
	_ = windows.SetSecurityInfo(h, windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION, nil, nil, merged, nil)
	return nil
}

// reopenSameObject opens f's path for DACL editing (WRITE_DAC|READ_CONTROL)
// without following a final reparse point, then confirms it is the same
// filesystem object as f (same volume + file id, not a reparse point). This
// defeats a symlink/path swap performed after the file was created.
func reopenSameObject(f *os.File) (windows.Handle, error) {
	namep, err := windows.UTF16PtrFromString(f.Name())
	if err != nil {
		return 0, err
	}
	h, err := windows.CreateFile(
		namep,
		windows.WRITE_DAC|windows.READ_CONTROL,
		windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE|windows.FILE_SHARE_DELETE,
		nil,
		windows.OPEN_EXISTING,
		windows.FILE_FLAG_OPEN_REPARSE_POINT|windows.FILE_FLAG_BACKUP_SEMANTICS,
		0,
	)
	if err != nil {
		return 0, err
	}
	var reopened, original windows.ByHandleFileInformation
	if err = windows.GetFileInformationByHandle(h, &reopened); err != nil {
		_ = windows.CloseHandle(h)
		return 0, err
	}
	if err = windows.GetFileInformationByHandle(windows.Handle(f.Fd()), &original); err != nil {
		_ = windows.CloseHandle(h)
		return 0, err
	}
	if reopened.VolumeSerialNumber != original.VolumeSerialNumber ||
		reopened.FileIndexHigh != original.FileIndexHigh ||
		reopened.FileIndexLow != original.FileIndexLow ||
		reopened.FileAttributes&windows.FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		_ = windows.CloseHandle(h)
		return 0, errors.New("config file identity mismatch (possible path swap)")
	}
	return h, nil
}
