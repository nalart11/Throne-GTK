//go:build !debug

package parentcheck

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

func CheckParentProcess() {
	if err := CheckGUIProcess(ParentPID); err != nil {
		log.Fatal(err)
	}
}

// CheckGUIProcess verifies that pid is the throne-gtk executable shipped next
// to this core. The macOS privileged helper uses this for the process at the
// other end of its control socket; the ordinary core uses it for its parent.
func CheckGUIProcess(pid int) error {
	parentPath, err := getParentExePath(pid)
	if err != nil {
		return fmt.Errorf("parent check: cannot read GUI executable: %w", err)
	}
	parentPath = resolveFinalPath(parentPath)

	selfPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("parent check: cannot read own executable: %w", err)
	}
	selfPath = resolveFinalPath(selfPath)

	selfDir := filepath.Dir(selfPath)
	parentDir := filepath.Dir(parentPath)
	parentBase := filepath.Base(parentPath)

	if runtime.GOOS == "windows" {
		if !strings.EqualFold(parentDir, selfDir) || !strings.EqualFold(parentBase, "throne-gtk.exe") {
			return fmt.Errorf("parent check failed: unexpected GUI %q, selfPath is %q", parentPath, selfPath)
		}
		return nil
	}

	if parentDir != selfDir || parentBase != "throne-gtk" {
		return fmt.Errorf("parent check failed: unexpected GUI %q, selfPath is %q", parentPath, selfPath)
	}
	return nil
}

func ProcessPath(pid int) (string, error) {
	path, err := getParentExePath(pid)
	if err != nil {
		return "", err
	}
	return resolveFinalPath(path), nil
}
