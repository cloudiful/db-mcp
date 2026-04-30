package database

import (
	"context"
	"fmt"
	"sort"
	"sync"

	"db/internal/config"
)

const (
	StatusUp       = "up"
	StatusDown     = "down"
	StatusDegraded = "degraded"
)

type Manager struct {
	connections map[string]*connectionSlot
	defaultName string
	limits      Limits
}

type connectionSlot struct {
	cfg     config.ConnectionConfig
	conn    Connection
	managed Connection
	limits  Limits

	mu     sync.RWMutex
	health ConnectionHealth
}

type managedConnection struct {
	slot *connectionSlot
}

var registry = map[string]OpenFunc{}

func Register(driver string, open OpenFunc) {
	registry[driver] = open
}

func NewManager(ctx context.Context, cfg *config.Config) (*Manager, error) {
	limits := NewLimits(cfg)
	connections := make(map[string]*connectionSlot, len(cfg.Connections))
	for _, connCfg := range cfg.Connections {
		open, ok := registry[connCfg.Driver]
		if !ok {
			closeSlots(connections)
			return nil, fmt.Errorf("driver %q is not registered", connCfg.Driver)
		}
		conn, err := open(ctx, connCfg, limits)
		if err != nil {
			closeSlots(connections)
			return nil, fmt.Errorf("open %s: %w", connCfg.Name, err)
		}
		slot := newConnectionSlot(connCfg, conn, limits)
		connections[connCfg.Name] = slot
	}

	manager := &Manager{
		connections: connections,
		defaultName: cfg.Connections[0].Name,
		limits:      limits,
	}
	manager.refreshAll(ctx)
	return manager, nil
}

func newConnectionSlot(cfg config.ConnectionConfig, conn Connection, limits Limits) *connectionSlot {
	info := conn.Info()
	slot := &connectionSlot{
		cfg:    cfg,
		conn:   conn,
		limits: limits,
		health: ConnectionHealth{
			Name:          info.Name,
			Driver:        info.Driver,
			DefaultSchema: info.DefaultSchema,
			Status:        StatusDown,
		},
	}
	slot.managed = &managedConnection{slot: slot}
	return slot
}

func closeSlots(connections map[string]*connectionSlot) {
	for _, slot := range connections {
		_ = slot.conn.Close()
	}
}

func (m *Manager) Limits() Limits {
	return m.limits
}

func (m *Manager) Connection(name string) (Connection, error) {
	slot, err := m.slot(name)
	if err != nil {
		return nil, err
	}
	return slot.managed, nil
}

func (m *Manager) ConnectionInfos() []ConnectionInfo {
	infos := make([]ConnectionInfo, 0, len(m.connections))
	for _, slot := range m.connections {
		infos = append(infos, slot.info())
	}
	sort.Slice(infos, func(i, j int) bool {
		return infos[i].Name < infos[j].Name
	})
	return infos
}

func (m *Manager) Health(ctx context.Context, name string, refresh bool) (*HealthSnapshot, error) {
	slots, err := m.slots(name)
	if err != nil {
		return nil, err
	}
	if refresh {
		var wg sync.WaitGroup
		for _, slot := range slots {
			wg.Add(1)
			go func(slot *connectionSlot) {
				defer wg.Done()
				_ = slot.refresh(ctx)
			}(slot)
		}
		wg.Wait()
	}

	connections := make([]ConnectionHealth, 0, len(slots))
	for _, slot := range slots {
		connections = append(connections, slot.snapshot())
	}
	sort.Slice(connections, func(i, j int) bool {
		return connections[i].Name < connections[j].Name
	})
	return &HealthSnapshot{
		Status:            overallStatus(connections),
		DefaultConnection: m.defaultName,
		Connections:       connections,
	}, nil
}

func (m *Manager) DefaultConnectionName() string {
	return m.defaultName
}

func (m *Manager) Close() error {
	var firstErr error
	for _, slot := range m.connections {
		if err := slot.conn.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func (m *Manager) refreshAll(ctx context.Context) {
	var wg sync.WaitGroup
	for _, slot := range m.connections {
		wg.Add(1)
		go func(slot *connectionSlot) {
			defer wg.Done()
			_ = slot.refresh(ctx)
		}(slot)
	}
	wg.Wait()
}

func (m *Manager) slot(name string) (*connectionSlot, error) {
	if name == "" {
		name = m.defaultName
	}
	slot, ok := m.connections[name]
	if !ok {
		return nil, fmt.Errorf("connection %q not found", name)
	}
	return slot, nil
}

func (m *Manager) slots(name string) ([]*connectionSlot, error) {
	if name != "" {
		slot, err := m.slot(name)
		if err != nil {
			return nil, err
		}
		return []*connectionSlot{slot}, nil
	}
	slots := make([]*connectionSlot, 0, len(m.connections))
	for _, slot := range m.connections {
		slots = append(slots, slot)
	}
	return slots, nil
}
