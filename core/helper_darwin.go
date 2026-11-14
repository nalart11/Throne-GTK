//go:build darwin

package main

import (
	"ThroneGtkCore/helpertrust"
	"ThroneGtkCore/internal/sys"
	"ThroneGtkCore/parentcheck"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"sync"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

const helperSocket = "/var/run/dev.nalart.ThroneGtk.helper.sock"

var admissions = make(chan struct{}, 16)
var workers = struct {
	sync.Mutex
	users map[int]bool
}{users: make(map[int]bool)}

func authenticateGUI(pid, uid int) error {
	if pid <= 0 || uid <= 0 {
		return errors.New("GUI must be a nonroot kernel-authenticated peer")
	}
	path, err := parentcheck.ProcessPath(pid)
	if err != nil {
		return err
	}
	if err = helpertrust.CheckLayout(helpertrust.Core, path); err != nil {
		return err
	}
	if err = helpertrust.CheckInstalled(); err != nil {
		return err
	}
	return helpertrust.CheckLive(pid)
}

func validateHelper() error {
	self, err := os.Executable()
	if err != nil {
		return err
	}
	if err = helpertrust.CheckLayout(self, helpertrust.GUI); err != nil {
		return err
	}
	return helpertrust.CheckInstalled()
}

func trustedGUIOverrideAllowed() bool {
	return os.Getuid() == 0 && os.Geteuid() == 0
}

func runPlatformMode() bool {
	if len(os.Args) < 2 {
		return false
	}
	switch os.Args[1] {
	case "--helper":
		runHelper()
		return true
	case "--helper-worker":
		runHelperWorker()
		return true
	case "--recover-dns":
		runRecoverDNS()
		return true
	default:
		return false
	}
}

// Fixed-purpose final recovery for uninstall, after launchd bootout and before
// removing the protected installation. No caller-supplied paths or operations.
func runRecoverDNS() {
	if len(os.Args) != 2 || os.Getuid() != 0 || os.Geteuid() != 0 {
		log.Fatal("DNS recovery requires root and accepts no additional arguments")
	}
	if err := validateHelper(); err != nil {
		log.Fatal("DNS recovery requires a trusted protected installation: ", err)
	}
	if err := sys.RecoverSystemDNS(); err != nil {
		log.Fatal("DNS recovery failed; protected recovery evidence retained: ", err)
	}
}

func runHelper() {
	if os.Getuid() != 0 || os.Geteuid() != 0 {
		log.Fatal("privileged helper must be started by launchd as root")
	}
	if err := validateHelper(); err != nil {
		log.Fatal("untrusted helper installation: ", err)
	}
	if err := helpertrust.CheckAncestors("/private/var/run", helpertrust.CheckNoACL); err != nil {
		log.Fatal("untrusted helper socket directory: ", err)
	}
	if err := os.Chdir("/"); err != nil {
		log.Fatal(err)
	}
	lock, err := lockHelperEndpoint()
	if err != nil {
		log.Fatal("helper singleton: ", err)
	}
	defer lock.Close() // Keep the lock inode permanently; never unlink a lock file.
	recoverHelperDNS()
	if err := helpertrust.RemoveStaleSocket(helperSocket); err != nil {
		log.Fatal("helper endpoint: ", err)
	}
	listener, err := net.Listen("unix", helperSocket)
	if err != nil {
		log.Fatal("helper listen: ", err)
	}
	defer listener.Close()
	if err = os.Chmod(helperSocket, 0o666); err != nil {
		log.Fatal("helper chmod: ", err)
	}
	for {
		conn, acceptErr := listener.Accept()
		if acceptErr != nil {
			log.Print("helper accept: ", acceptErr)
			continue
		}
		select {
		case admissions <- struct{}{}:
			go func() { defer func() { <-admissions }(); handleHelperRequest(conn.(*net.UnixConn)) }()
		default:
			conn.Close()
		}
	}
}

func handleHelperRequest(conn *net.UnixConn) {
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(5 * time.Second))
	pid, uid, gid, err := helperPeer(conn)
	if err == nil {
		err = authenticateGUI(pid, uid)
	}
	workerPID := 0
	var header [15]byte
	if err == nil {
		err = authenticateGUI(pid, uid)
	}
	if err == nil {
		workers.Lock()
		if workers.users[uid] {
			err = errors.New("worker already running for this UID")
		} else {
			workers.users[uid] = true
		}
		workers.Unlock()
	}
	reserved := err == nil
	if err == nil {
		_, err = io.ReadFull(conn, header[:])
	}
	if err == nil && string(header[:8]) != "THRONE1\x00" {
		err = errors.New("invalid helper request magic")
	}
	claimedPID := int(binary.LittleEndian.Uint32(header[8:12]))
	debug := header[12] != 0
	pathLen := int(binary.LittleEndian.Uint16(header[13:15]))
	if err == nil && (claimedPID != pid || pathLen == 0 || pathLen > 4096) {
		err = errors.New("invalid helper request")
	}
	pathBytes := make([]byte, pathLen)
	if err == nil {
		_, err = io.ReadFull(conn, pathBytes)
	}
	endpoint := string(pathBytes)
	if err == nil {
		err = validateGUIEndpoint(endpoint, uid)
	}
	if err == nil {
		err = authenticateGUI(pid, uid)
	}
	if err == nil {
		self, executableErr := os.Executable()
		if executableErr != nil {
			err = executableErr
		} else {
			cmd := exec.Command(self, "--helper-worker")
			cmd.Dir = "/"
			cmd.Env = append([]string{"PATH=/usr/bin:/bin:/usr/sbin:/sbin", "HOME=/var/root", "TMPDIR=/private/tmp"},
				"THRONE_CORE_SOCKET="+endpoint,
				"THRONE_CORE_GUI_PID="+strconv.Itoa(pid),
				"THRONE_CORE_CLIENT_UID="+strconv.Itoa(uid),
				"THRONE_CORE_CLIENT_GID="+strconv.Itoa(gid),
				fmt.Sprintf("THRONE_CORE_DEBUG=%d", boolInt(debug)),
			)
			err = cmd.Start()
			if err == nil {
				workerPID = cmd.Process.Pid
				go func() {
					if err := cmd.Wait(); err != nil {
						log.Print("helper worker exited abnormally")
					}
					// The dead worker's flock has been released. Recover before
					// making this UID available to another worker request.
					recoverHelperDNS()
					workers.Lock()
					delete(workers.users, uid)
					workers.Unlock()
				}()
			}
		}
	}
	if reserved && err != nil {
		workers.Lock()
		delete(workers.users, uid)
		workers.Unlock()
	}
	if err := writeHelperResponse(conn, err, workerPID); err != nil {
		log.Print("helper response: ", err)
	}
}

func recoverHelperDNS() {
	if err := sys.RecoverSystemDNS(); err != nil && !errors.Is(err, sys.ErrDNSLeaseBusy) {
		// Recovery errors contain fixed-path/validation/status diagnostics, not
		// the original or intended DNS dictionaries. Keep the actionable cause.
		log.Print("helper DNS recovery failed; protected recovery evidence retained: ", err)
	}
}

// The caller has already verified every ancestor of this fixed path. O_NOFOLLOW
// and post-open inode/ownership checks prevent trusting a substituted lock file.
func lockHelperEndpoint() (*os.File, error) {
	const path = "/private/var/run/dev.nalart.ThroneGtk.helper.lock"
	fd, err := unix.Open(path, unix.O_RDWR|unix.O_CREAT|unix.O_NOFOLLOW|unix.O_CLOEXEC|unix.O_NONBLOCK, 0600)
	if err != nil {
		return nil, err
	}
	f := os.NewFile(uintptr(fd), path)
	fail := func(err error) (*os.File, error) { f.Close(); return nil, err }
	info, err := f.Stat()
	if err != nil {
		return fail(err)
	}
	st, ok := info.Sys().(*syscall.Stat_t)
	if !ok || !info.Mode().IsRegular() || st.Uid != 0 || st.Nlink != 1 || info.Mode().Perm() != 0600 {
		return fail(errors.New("unsafe helper lock ownership/type/mode/link count"))
	}
	if err := helpertrust.CheckNoACL(path); err != nil {
		return fail(err)
	}
	if err := unix.Flock(fd, unix.LOCK_EX|unix.LOCK_NB); err != nil {
		return fail(fmt.Errorf("another helper holds singleton lock: %w", err))
	}
	current, err := os.Lstat(path)
	if err != nil {
		return fail(err)
	}
	if !os.SameFile(info, current) {
		return fail(errors.New("helper lock inode changed"))
	}
	return f, nil
}

func runHelperWorker() {
	if os.Getuid() != 0 || os.Geteuid() != 0 {
		log.Fatal("helper worker must run as real and effective root")
	}
	if err := validateHelper(); err != nil {
		log.Fatal(err)
	}
	self, err := os.Executable()
	if err != nil {
		log.Fatal(err)
	}
	self, _ = filepath.EvalSymlinks(self)
	parent, parentErr := parentcheck.ProcessPath(os.Getppid())
	if parentErr == nil {
		parent, parentErr = filepath.EvalSymlinks(parent)
	}
	if parentErr != nil || filepath.Clean(parent) != filepath.Clean(self) {
		log.Fatalf("helper worker has unexpected parent %q: %v", parent, parentErr)
	}
	if err := os.Chdir("/"); err != nil {
		log.Fatal(err)
	}
	pid, err := strconv.Atoi(os.Getenv("THRONE_CORE_GUI_PID"))
	if err != nil {
		log.Fatal(err)
	}
	uid, err := strconv.Atoi(os.Getenv("THRONE_CORE_CLIENT_UID"))
	if err != nil {
		log.Fatal(err)
	}
	endpoint := os.Getenv("THRONE_CORE_SOCKET")
	if err := validateGUIEndpoint(endpoint, uid); err != nil {
		log.Fatal(err)
	}
	c, err := net.DialTimeout("unix", endpoint, 5*time.Second)
	if err != nil {
		log.Fatal(err)
	}
	defer c.Close()
	peerPID, peerUID, _, err := helperPeer(c.(*net.UnixConn))
	if err != nil || peerPID != pid || peerUID != uid {
		log.Fatal("worker IPC peer mismatch: ", err)
	}
	if err := authenticateGUI(peerPID, peerUID); err != nil {
		log.Fatal(err)
	}
	// Enabled only after the privileged bootstrap and connected GUI peer have
	// passed mandatory authentication. Ordinary core/test paths remain unchanged.
	enablePrivilegedWorkerPolicy()
	debug = os.Getenv("THRONE_CORE_DEBUG") == "1" // logging only; authentication above is unconditional.
	go func() {
		for {
			time.Sleep(5 * time.Second)
			if err := syscall.Kill(pid, 0); err != nil {
				_ = c.Close() // Unblock dispatch so its normal deferred cleanup runs.
				return
			}
		}
	}()
	prepareCoreRuntime()
	runDispatch(c)
}

func helperPeer(conn *net.UnixConn) (pid, uid, gid int, err error) {
	raw, err := conn.SyscallConn()
	if err != nil {
		return 0, 0, 0, err
	}
	controlErr := raw.Control(func(fd uintptr) {
		pid, err = unix.GetsockoptInt(int(fd), unix.SOL_LOCAL, unix.LOCAL_PEERPID)
		if err != nil {
			return
		}
		var cred *unix.Xucred
		cred, err = unix.GetsockoptXucred(int(fd), unix.SOL_LOCAL, unix.LOCAL_PEERCRED)
		if err == nil {
			uid = int(cred.Uid)
			if cred.Ngroups > 0 {
				gid = int(cred.Groups[0])
			}
		}
	})
	if controlErr != nil {
		err = controlErr
	}
	return
}

func validateGUIEndpoint(path string, uid int) error {
	if !filepath.IsAbs(path) {
		return errors.New("GUI socket path is not absolute")
	}
	// Canonicalize only the documented macOS /var alias, not arbitrary symlinks.
	if len(path) >= 5 && path[:5] == "/var/" {
		path = "/private" + path
	}
	parent := filepath.Dir(path)
	resolved, err := filepath.EvalSymlinks(parent)
	if err != nil || resolved != parent {
		return errors.New("GUI endpoint parent contains symlinks")
	}
	pinfo, err := os.Lstat(parent)
	if err != nil {
		return err
	}
	pstat, ok := pinfo.Sys().(*syscall.Stat_t)
	if !ok || !pinfo.IsDir() || int(pstat.Uid) != uid || pinfo.Mode().Perm()&0077 != 0 {
		return errors.New("GUI endpoint requires a private user-owned directory")
	}
	if err := helpertrust.CheckNoACL(parent); err != nil {
		return err
	}
	info, err := os.Lstat(path)
	if err != nil {
		return fmt.Errorf("GUI socket: %w", err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok || int(stat.Uid) != uid || info.Mode()&os.ModeSocket == 0 {
		return errors.New("GUI endpoint is not a socket owned by the requesting user")
	}
	return nil
}

func writeHelperResponse(conn net.Conn, requestErr error, pid int) error {
	payload, _ := json.Marshal(struct {
		Version int  `json:"version"`
		OK      bool `json:"ok"`
		PID     int  `json:"pid"`
	}{1, requestErr == nil, pid})
	message := string(payload)
	status := byte(0)
	if requestErr != nil {
		status = 1
		message = requestErr.Error()
	}
	if len(message) > 65535 {
		message = message[:65535]
	}
	header := []byte{status, 0, 0}
	binary.LittleEndian.PutUint16(header[1:], uint16(len(message)))
	return helpertrust.WriteFull(conn, append(header, message...))
}

func boolInt(value bool) int {
	if value {
		return 1
	}
	return 0
}
