// Package helpertrust implements mandatory privileged-helper policy, including
// in debug builds. It deliberately does not use parentcheck.CheckGUIProcess.
package helpertrust

import (
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"syscall"
)

const Bundle = "/Library/Application Support/Nolurix/Nolurix.app"
const GUI = Bundle + "/Contents/MacOS/nolurix"
const Core = Bundle + "/Contents/MacOS/nolurix-core"

func CheckNode(path string, info fs.FileInfo, owner uint32) error {
	if info.Mode()&os.ModeSymlink != 0 {
		return fmt.Errorf("untrusted symlink: %s", path)
	}
	if owner != 0 || info.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("%s must be root-owned and not group/world writable; reinstall using the PKG", path)
	}
	return nil
}

// CheckTree includes replacement-sensitive ancestors and every bundle resource.
// acl must fail closed when ACL metadata cannot be read.
func CheckTree(bundle string, acl func(string) error) error {
	for p := filepath.Dir(bundle); ; p = filepath.Dir(p) {
		if err := checkPath(p, acl); err != nil {
			return err
		}
		if p == "/" {
			break
		}
	}
	return filepath.WalkDir(bundle, func(p string, _ fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		info, err := os.Lstat(p)
		if err != nil {
			return err
		}
		if info.Mode()&os.ModeSymlink != 0 {
			return CheckBundleLink(bundle, p)
		}
		return checkPath(p, acl)
	})
}

// Framework version links and dylib aliases are allowed only inside Frameworks.
// EvalSymlinks rejects dangling links and cycles; the resolved closure is walked
// independently, including its ownership, permissions and ACLs.
func CheckBundleLink(bundle, path string) error {
	frameworks := filepath.Join(bundle, "Contents", "Frameworks")
	within := func(root, p string) bool {
		rel, err := filepath.Rel(root, p)
		return err == nil && rel != ".." && !filepath.IsAbs(rel) && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
	}
	if !within(frameworks, path) {
		return fmt.Errorf("symlink outside Frameworks: %s", path)
	}
	target, err := os.Readlink(path)
	if err != nil {
		return err
	}
	if !filepath.IsAbs(target) {
		target = filepath.Join(filepath.Dir(path), target)
	}
	if !within(frameworks, filepath.Clean(target)) {
		return fmt.Errorf("escaping framework link target: %s", path)
	}
	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		return err
	}
	// Compare canonical identities on both sides: macOS temporary paths can
	// use /var while EvalSymlinks returns /private/var. This does not relax
	// CheckTree's independent rejection of symlinked installation ancestors.
	resolvedFrameworks, err := filepath.EvalSymlinks(frameworks)
	if err != nil {
		return err
	}
	if !within(resolvedFrameworks, resolved) {
		return fmt.Errorf("escaping framework symlink: %s", path)
	}
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	st, ok := info.Sys().(*syscall.Stat_t)
	if !ok || st.Uid != 0 {
		return fmt.Errorf("untrusted symlink owner: %s", path)
	}
	return nil
}

func checkPath(path string, acl func(string) error) error {
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	st, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		return fmt.Errorf("missing ownership: %s", path)
	}
	if err = CheckNode(path, info, st.Uid); err != nil {
		return err
	}
	if !info.IsDir() && !info.Mode().IsRegular() {
		return fmt.Errorf("unexpected bundle object: %s", path)
	}
	return acl(path)
}

func CheckLayout(self, gui string) error {
	if filepath.Clean(self) != self || filepath.Clean(gui) != gui {
		return fmt.Errorf("noncanonical executable path")
	}
	if self == Core && gui == GUI {
		return nil
	}
	var err error
	self, err = filepath.EvalSymlinks(self)
	if err != nil {
		return err
	}
	gui, err = filepath.EvalSymlinks(gui)
	if err != nil {
		return err
	}
	if self != Core || gui != GUI {
		return fmt.Errorf("helper requires fixed installed layout %s", Bundle)
	}
	return nil
}

func CheckAncestors(path string, acl func(string) error) error {
	for p := path; ; p = filepath.Dir(p) {
		if err := checkPath(p, acl); err != nil {
			return err
		}
		if p == "/" {
			return nil
		}
	}
}
