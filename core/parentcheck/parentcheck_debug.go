//go:build debug

package parentcheck

func CheckParentProcess() {}

func CheckGUIProcess(int) error { return nil }

func ProcessPath(pid int) (string, error) {
	path, err := getParentExePath(pid)
	if err != nil {
		return "", err
	}
	return resolveFinalPath(path), nil
}
