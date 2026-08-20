package parentcheck

import "os"

// ParentPID is the parent process ID captured once at package init.
// All subsequent parent-identity checks (executable path, peer pid on the IPC
// socket, parent-death watcher) read this value so they cannot disagree if the
// real parent dies and getppid() starts returning a different pid.
var ParentPID = os.Getppid()
