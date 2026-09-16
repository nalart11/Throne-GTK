//go:build darwin

package main

import (
	"ThroneGtkCore/parentcheck"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"syscall"

	"golang.org/x/sys/unix"
)

const helperSocket = "/var/run/dev.nalart.ThroneGtk.helper.sock"

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
	default:
		return false
	}
}

func runHelper() {
	if os.Getuid() != 0 || os.Geteuid() != 0 {
		log.Fatal("privileged helper must be started by launchd as root")
	}
	_ = os.Remove(helperSocket)
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
		go handleHelperRequest(conn.(*net.UnixConn))
	}
}

func handleHelperRequest(conn *net.UnixConn) {
	defer conn.Close()
	pid, uid, gid, err := helperPeer(conn)
	if err == nil {
		err = parentcheck.CheckGUIProcess(pid)
	}
	var header [15]byte
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
		self, executableErr := os.Executable()
		if executableErr != nil {
			err = executableErr
		} else {
			cmd := exec.Command(self, "--helper-worker")
			cmd.Env = append(os.Environ(),
				"THRONE_CORE_SOCKET="+endpoint,
				"THRONE_CORE_GUI_PID="+strconv.Itoa(pid),
				"THRONE_CORE_CLIENT_UID="+strconv.Itoa(uid),
				"THRONE_CORE_CLIENT_GID="+strconv.Itoa(gid),
				fmt.Sprintf("THRONE_CORE_DEBUG=%d", boolInt(debug)),
			)
			err = cmd.Start()
			if err == nil {
				go func() { _ = cmd.Wait() }()
			}
		}
	}
	writeHelperResponse(conn, err)
}

func runHelperWorker() {
	if os.Getuid() != 0 || os.Geteuid() != 0 {
		log.Fatal("helper worker must run as real and effective root")
	}
	self, err := os.Executable()
	if err != nil {
		log.Fatal(err)
	}
	self, _ = filepath.EvalSymlinks(self)
	parent, parentErr := parentcheck.ProcessPath(os.Getppid())
	if parentErr != nil || filepath.Clean(parent) != filepath.Clean(self) {
		log.Fatalf("helper worker has unexpected parent %q: %v", parent, parentErr)
	}
	prepareCoreRuntime()
	RunCore()
}

func helperPeer(conn *net.UnixConn) (pid, uid, gid int, err error) {
	raw, err := conn.SyscallConn()
	if err != nil {
		return 0, 0, 0, err
	}
	err = raw.Control(func(fd uintptr) {
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
	return
}

func validateGUIEndpoint(path string, uid int) error {
	if !filepath.IsAbs(path) {
		return errors.New("GUI socket path is not absolute")
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

func writeHelperResponse(conn net.Conn, requestErr error) {
	message := "ok"
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
	_, _ = conn.Write(append(header, message...))
}

func boolInt(value bool) int {
	if value {
		return 1
	}
	return 0
}
