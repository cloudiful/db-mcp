package database

import (
	"context"
	"fmt"
	"sort"
	"sync"
	"time"

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

func (s *connectionSlot) info() ConnectionInfo {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return ConnectionInfo{
		Name:             s.health.Name,
		Driver:           s.health.Driver,
		DefaultSchema:    s.health.DefaultSchema,
		Status:           s.health.Status,
		Available:        s.health.Available,
		LastCheckedAt:    copyTimePtr(s.health.LastCheckedAt),
		LastSuccessfulAt: copyTimePtr(s.health.LastSuccessfulAt),
		LastError:        s.health.LastError,
	}
}

func (s *connectionSlot) snapshot() ConnectionHealth {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return ConnectionHealth{
		Name:             s.health.Name,
		Driver:           s.health.Driver,
		DefaultSchema:    s.health.DefaultSchema,
		Status:           s.health.Status,
		Available:        s.health.Available,
		LastCheckedAt:    copyTimePtr(s.health.LastCheckedAt),
		LastSuccessfulAt: copyTimePtr(s.health.LastSuccessfulAt),
		LastError:        s.health.LastError,
	}
}

func (s *connectionSlot) refresh(ctx context.Context) error {
	if err := s.conn.Ping(ctx); err != nil {
		s.markDown(err)
		return err
	}
	s.markUp()
	return nil
}

func (s *connectionSlot) ensureReady(ctx context.Context) error {
	s.mu.RLock()
	available := s.health.Available
	s.mu.RUnlock()
	if available {
		return nil
	}
	if err := s.refresh(ctx); err != nil {
		return unavailableError(s.health.Name, err)
	}
	return nil
}

func (s *connectionSlot) markUp() {
	info := s.conn.Info()
	now := time.Now().UTC()

	s.mu.Lock()
	defer s.mu.Unlock()
	s.health.Name = info.Name
	s.health.Driver = info.Driver
	s.health.DefaultSchema = info.DefaultSchema
	s.health.Status = StatusUp
	s.health.Available = true
	s.health.LastCheckedAt = copyTimePtr(&now)
	s.health.LastSuccessfulAt = copyTimePtr(&now)
	s.health.LastError = ""
}

func (s *connectionSlot) markDown(err error) {
	info := s.conn.Info()
	now := time.Now().UTC()

	s.mu.Lock()
	defer s.mu.Unlock()
	s.health.Name = info.Name
	s.health.Driver = info.Driver
	s.health.DefaultSchema = info.DefaultSchema
	s.health.Status = StatusDown
	s.health.Available = false
	s.health.LastCheckedAt = copyTimePtr(&now)
	if err != nil {
		s.health.LastError = err.Error()
	}
}

func (s *connectionSlot) handleOperationError(err error) error {
	pingCtx, cancel := context.WithTimeout(context.Background(), s.limits.QueryTimeout)
	defer cancel()
	if pingErr := s.conn.Ping(pingCtx); pingErr != nil {
		s.markDown(pingErr)
		return unavailableError(s.conn.Info().Name, pingErr)
	}
	s.markUp()
	return err
}

func (c *managedConnection) Info() ConnectionInfo {
	return c.slot.info()
}

func (c *managedConnection) Ping(ctx context.Context) error {
	if err := c.slot.refresh(ctx); err != nil {
		return unavailableError(c.slot.conn.Info().Name, err)
	}
	return nil
}

func (c *managedConnection) ListSchemas(ctx context.Context, filter string, limit int) ([]SchemaInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListSchemas(ctx, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListTables(ctx context.Context, schema, filter string, limit int) ([]TableInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListTables(ctx, schema, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]ColumnMatchInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.SearchColumns(ctx, schema, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListForeignKeys(ctx context.Context, schema, table string, limit int) ([]ForeignKeyInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListForeignKeys(ctx, schema, table, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]IndexInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListIndexes(ctx, schema, table, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) GetTableComments(ctx context.Context, schema, table string) (*TableCommentInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.GetTableComments(ctx, schema, table)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) DescribeTable(ctx context.Context, schema, table string) ([]ColumnInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.DescribeTable(ctx, schema, table)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*QueryResult, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ExecuteQuery(ctx, query, maxRows)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ExplainQuery(ctx context.Context, query string) (*ExplainResult, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ExplainQuery(ctx, query)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) Close() error {
	return nil
}

func overallStatus(connections []ConnectionHealth) string {
	if len(connections) == 0 {
		return StatusDown
	}
	available := 0
	for _, connection := range connections {
		if connection.Available {
			available++
		}
	}
	switch {
	case available == 0:
		return StatusDown
	case available == len(connections):
		return StatusUp
	default:
		return StatusDegraded
	}
}

func unavailableError(name string, err error) error {
	return fmt.Errorf("connection %q is unavailable: %w", name, err)
}

func copyTimePtr(value *time.Time) *time.Time {
	if value == nil {
		return nil
	}
	copied := *value
	return &copied
}
