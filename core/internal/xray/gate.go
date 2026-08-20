package xray

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"strconv"
	"sync"
	"sync/atomic"
	"time"

	"github.com/xtls/xray-core/core"
)

// How often the gate looks for an instance that has gone idle, and the floor on
// that once a short idle window pulls it down.
const (
	gateSweepInterval    = 15 * time.Second
	minGateSweepInterval = 2 * time.Second
)

// Keeps shutdown within about half the idle window instead of a fixed tick late.
func sweepIntervalFor(idle time.Duration) time.Duration {
	if idle <= 0 || idle/2 >= gateSweepInterval {
		return gateSweepInterval
	}
	if half := idle / 2; half > minGateSweepInterval {
		return half
	}
	return minGateSweepInterval
}

// Gate keeps an Xray instance cold until traffic actually needs it.
//
// An auto-selector pool can hold Xray members that sit in the bench tier and
// get dialed once every bench cycle; keeping a live instance (plus a handler
// per member) resident the whole time is pure overhead. The gate takes over the
// loopback ports sing-box was told to dial, moves Xray's own inbounds onto
// fresh ports behind them, and only builds the instance once a connection
// arrives.
//
// Owning the listening socket for the whole life of the config is the point: a
// dial that lands while the instance is down must not hit a closed port, which
// the auto-selector would read as a dead member rather than a cold one.
//
// UDP needs no gate of its own. Xray answers a SOCKS5 UDP ASSOCIATE with a
// per-session ephemeral hub on its own address, so datagrams never touch the
// inbound port to begin with. The association's TCP control connection does run
// through the gate and counts as active traffic, so the instance can never be
// shut down out from under a live UDP session.
type Gate struct {
	config  string
	idle    time.Duration
	prepare func(*core.Instance) error

	bridges []gateBridge
	stopped chan struct{}

	mu       sync.Mutex
	instance *core.Instance
	closed   bool

	// conns is nil once the gate is closed, which is also how a connection
	// accepted during teardown learns to give up.
	connsMu sync.Mutex
	conns   map[net.Conn]struct{}

	active  atomic.Int64
	lastUse atomic.Int64 // unix nanos
}

type gateBridge struct {
	listener net.Listener
	target   string
}

type gateAddrPair struct{ gate, target string }

// StartGate moves every inbound in the config onto a fresh port, takes over the
// ports they used to listen on, and returns a gate with no instance running
// yet. The config is validated up front so a broken profile still fails at
// Start time rather than on some later dial. prepare runs on each instance
// before it starts and must apply everything the caller would have applied to
// an eagerly created one (egress interface, outbound DNS). An idle of 0 keeps
// the instance resident once it has started.
func StartGate(config string, idle time.Duration, prepare func(*core.Instance) error) (*Gate, error) {
	rewritten, pairs, err := remapInbounds(config)
	if err != nil {
		return nil, err
	}
	if err = CheckXrayConfig(rewritten); err != nil {
		return nil, err
	}

	gate := &Gate{
		config:  rewritten,
		idle:    idle,
		prepare: prepare,
		stopped: make(chan struct{}),
		conns:   make(map[net.Conn]struct{}),
	}
	gate.touch()

	for _, pair := range pairs {
		listener, listenErr := net.Listen("tcp", pair.gate)
		if listenErr != nil {
			for _, bridge := range gate.bridges {
				_ = bridge.listener.Close()
			}
			return nil, fmt.Errorf("xray gate: cannot take over %s: %w", pair.gate, listenErr)
		}
		gate.bridges = append(gate.bridges, gateBridge{listener: listener, target: pair.target})
	}

	for i := range gate.bridges {
		go gate.serve(&gate.bridges[i])
	}
	go gate.sweep()
	return gate, nil
}

// remapInbounds rewrites each inbound onto a free port on the same address and
// reports the (original, new) address pairs. Only the inbounds array is decoded
// and re-encoded, so the rest of the config — including any hand-written
// outbound JSON — reaches Xray byte for byte as it arrived.
func remapInbounds(config string) (string, []gateAddrPair, error) {
	var root map[string]json.RawMessage
	if err := json.Unmarshal([]byte(config), &root); err != nil {
		return "", nil, fmt.Errorf("xray gate: parse config: %w", err)
	}
	var inbounds []map[string]any
	if raw, ok := root["inbounds"]; ok {
		if err := json.Unmarshal(raw, &inbounds); err != nil {
			return "", nil, fmt.Errorf("xray gate: parse inbounds: %w", err)
		}
	}
	if len(inbounds) == 0 {
		return "", nil, errors.New("xray gate: config has no inbounds to gate")
	}

	// Hold every probe open until the whole set is chosen: released ephemeral
	// ports get handed straight back out, so probing one at a time can deal the
	// same port to two inbounds.
	var probes []net.Listener
	defer func() {
		for _, probe := range probes {
			_ = probe.Close()
		}
	}()

	pairs := make([]gateAddrPair, 0, len(inbounds))
	for _, inbound := range inbounds {
		host, _ := inbound["listen"].(string)
		if host == "" {
			host = "127.0.0.1"
		}
		port, ok := inbound["port"].(float64)
		if !ok || port <= 0 {
			// Every inbound has to be gated. One left on its own port would
			// take connections while the instance is down, which is exactly the
			// closed-port failure the gate exists to prevent.
			return "", nil, errors.New("xray gate: inbound without a fixed port cannot be gated")
		}
		probe, err := net.Listen("tcp", net.JoinHostPort(host, "0"))
		if err != nil {
			return "", nil, fmt.Errorf("xray gate: no free port on %s: %w", host, err)
		}
		probes = append(probes, probe)
		moved := probe.Addr().(*net.TCPAddr).Port
		inbound["port"] = moved
		pairs = append(pairs, gateAddrPair{
			gate:   net.JoinHostPort(host, strconv.Itoa(int(port))),
			target: net.JoinHostPort(host, strconv.Itoa(moved)),
		})
	}

	patched, err := json.Marshal(inbounds)
	if err != nil {
		return "", nil, err
	}
	root["inbounds"] = patched
	out, err := json.Marshal(root)
	if err != nil {
		return "", nil, err
	}
	return string(out), pairs, nil
}

// Instance is the live instance, or nil while the gate is cold. Callers use it
// to push runtime changes (the egress interface) onto whatever is up now.
func (g *Gate) Instance() *core.Instance {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.instance
}

func (g *Gate) Close() {
	g.mu.Lock()
	if g.closed {
		g.mu.Unlock()
		return
	}
	g.closed = true
	instance := g.instance
	g.instance = nil
	g.mu.Unlock()

	close(g.stopped)
	for _, bridge := range g.bridges {
		_ = bridge.listener.Close()
	}

	g.connsMu.Lock()
	conns := g.conns
	g.conns = nil
	g.connsMu.Unlock()
	for conn := range conns {
		_ = conn.Close()
	}

	if instance != nil {
		_ = instance.Close()
	}
}

func (g *Gate) serve(bridge *gateBridge) {
	for {
		conn, err := bridge.listener.Accept()
		if err != nil {
			return
		}
		go g.handle(conn, bridge.target)
	}
}

func (g *Gate) handle(conn net.Conn, target string) {
	if !g.track(conn) {
		_ = conn.Close()
		return
	}
	defer func() {
		g.forget(conn)
		_ = conn.Close()
	}()

	// Counted before the instance is even up: the sweeper must not decide the
	// gate is idle while a connection is still being set up.
	g.active.Add(1)
	g.touch()
	defer func() {
		g.active.Add(-1)
		g.touch()
	}()

	if _, err := g.ensure(); err != nil {
		log.Println("Xray gate: failed to bring up the instance:", err)
		return
	}
	upstream, err := net.Dial("tcp", target)
	if err != nil {
		log.Println("Xray gate: failed to reach the instance:", err)
		return
	}
	if !g.track(upstream) {
		_ = upstream.Close()
		return
	}
	defer func() {
		g.forget(upstream)
		_ = upstream.Close()
	}()

	done := make(chan struct{}, 2)
	go relay(upstream, conn, done)
	go relay(conn, upstream, done)
	<-done
	<-done
}

// relay copies until EOF and then half-closes the destination, so a peer that
// has finished sending can still receive. Closing both ends on the first EOF
// would truncate any download whose request body ended early.
func relay(dst, src net.Conn, done chan<- struct{}) {
	defer func() { done <- struct{}{} }()
	_, _ = io.Copy(dst, src)
	if closer, ok := dst.(interface{ CloseWrite() error }); ok {
		_ = closer.CloseWrite()
	}
}

func (g *Gate) ensure() (*core.Instance, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.closed {
		return nil, errors.New("xray gate: closed")
	}
	if g.instance != nil {
		return g.instance, nil
	}
	instance, err := CreateXrayInstance(g.config)
	if err != nil {
		return nil, err
	}
	if g.prepare != nil {
		if err = g.prepare(instance); err != nil {
			_ = instance.Close()
			return nil, err
		}
	}
	if err = instance.Start(); err != nil {
		_ = instance.Close()
		return nil, err
	}
	log.Println("Xray gate: started the instance on demand")
	g.instance = instance
	return instance, nil
}

func (g *Gate) sweep() {
	ticker := time.NewTicker(sweepIntervalFor(g.idle))
	defer ticker.Stop()
	for {
		select {
		case <-g.stopped:
			return
		case <-ticker.C:
			if g.idle <= 0 || g.active.Load() > 0 {
				continue
			}
			if time.Since(time.Unix(0, g.lastUse.Load())) < g.idle {
				continue
			}
			g.mu.Lock()
			instance := g.instance
			g.instance = nil
			g.mu.Unlock()
			if instance != nil {
				log.Println("Xray gate: the instance went idle, shutting it down")
				_ = instance.Close()
			}
		}
	}
}

func (g *Gate) touch() {
	g.lastUse.Store(time.Now().UnixNano())
}

// track registers a connection so Close can tear it down, and reports false
// once the gate is closed and nothing will come back for it.
func (g *Gate) track(conn net.Conn) bool {
	g.connsMu.Lock()
	defer g.connsMu.Unlock()
	if g.conns == nil {
		return false
	}
	g.conns[conn] = struct{}{}
	return true
}

func (g *Gate) forget(conn net.Conn) {
	g.connsMu.Lock()
	defer g.connsMu.Unlock()
	delete(g.conns, conn)
}
