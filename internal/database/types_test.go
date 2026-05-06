package database

import (
	"context"
	"errors"
	"sync"
	"testing"

	"db/internal/config"
)

type stubConnection struct {
	mu      sync.Mutex
	name    string
	driver  string
	schema  string
	pingErr error
	closed  bool
}

func (s *stubConnection) Info() ConnectionInfo {
	s.mu.Lock()
	defer s.mu.Unlock()
	return ConnectionInfo{Name: s.name, Driver: s.driver, DefaultSchema: s.schema}
}

func (s *stubConnection) Ping(context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pingErr
}

func (s *stubConnection) ListSchemas(context.Context, string, int) ([]SchemaInfo, error) {
	return []SchemaInfo{{Name: "main"}}, nil
}

func (s *stubConnection) ListTables(context.Context, string, string, int) ([]TableInfo, error) {
	return nil, nil
}

func (s *stubConnection) SearchColumns(context.Context, string, string, int) ([]ColumnMatchInfo, error) {
	return nil, nil
}

func (s *stubConnection) ListConstraints(context.Context, string, string, int) ([]ConstraintInfo, error) {
	return nil, nil
}

func (s *stubConnection) ListIndexes(context.Context, string, string, int) ([]IndexInfo, error) {
	return nil, nil
}

func (s *stubConnection) GetTableComments(context.Context, string, string) (*TableCommentInfo, error) {
	return nil, nil
}

func (s *stubConnection) DescribeTable(context.Context, string, string) ([]ColumnInfo, error) {
	return nil, nil
}

func (s *stubConnection) GetTableDDL(context.Context, string, string) (*TableDDLInfo, error) {
	return nil, nil
}

func (s *stubConnection) ExecuteQuery(context.Context, string, int) (*QueryResult, error) {
	return nil, nil
}

func (s *stubConnection) ExplainQuery(context.Context, string) (*ExplainResult, error) {
	return nil, nil
}

func (s *stubConnection) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.closed = true
	return nil
}

func (s *stubConnection) setPingErr(err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.pingErr = err
}

func (s *stubConnection) isClosed() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.closed
}

func TestNewManagerClosesOpenedConnectionsOnFatalOpenFailure(t *testing.T) {
	primary := &stubConnection{name: "primary", driver: "stub", schema: "main"}
	primaryDriver := "manager-cleanup-primary"
	failingDriver := "manager-cleanup-failing"

	Register(primaryDriver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return primary, nil
	})
	Register(failingDriver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return nil, errors.New("boom")
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "primary", Driver: primaryDriver},
			{Name: "secondary", Driver: failingDriver},
		},
	}

	_, err := NewManager(context.Background(), cfg)
	if err == nil {
		t.Fatalf("expected manager creation to fail")
	}
	if !primary.isClosed() {
		t.Fatalf("expected opened connections to be closed on failure")
	}
}

func TestNewManagerStartsWithMixedHealth(t *testing.T) {
	healthy := &stubConnection{name: "healthy", driver: "stub", schema: "main"}
	unhealthy := &stubConnection{name: "unhealthy", driver: "stub", schema: "main", pingErr: errors.New("dial tcp timeout")}
	healthyDriver := "manager-health-healthy"
	unhealthyDriver := "manager-health-unhealthy"

	Register(healthyDriver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return healthy, nil
	})
	Register(unhealthyDriver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return unhealthy, nil
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "healthy", Driver: healthyDriver},
			{Name: "unhealthy", Driver: unhealthyDriver},
		},
	}

	manager, err := NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	infoByName := map[string]ConnectionInfo{}
	for _, info := range manager.ConnectionInfos() {
		infoByName[info.Name] = info
	}
	if !infoByName["healthy"].Available || infoByName["healthy"].Status != StatusUp {
		t.Fatalf("expected healthy connection up, got %#v", infoByName["healthy"])
	}
	if infoByName["unhealthy"].Available || infoByName["unhealthy"].Status != StatusDown {
		t.Fatalf("expected unhealthy connection down, got %#v", infoByName["unhealthy"])
	}
	if infoByName["unhealthy"].LastError == "" {
		t.Fatalf("expected unhealthy connection to record last error")
	}
}

func TestNewManagerStartsWhenAllConnectionsAreDown(t *testing.T) {
	down := &stubConnection{name: "down", driver: "stub", schema: "main", pingErr: errors.New("db down")}
	driver := "manager-all-down"

	Register(driver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return down, nil
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "down", Driver: driver},
		},
	}

	manager, err := NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	snapshot, err := manager.Health(context.Background(), "", false)
	if err != nil {
		t.Fatalf("health: %v", err)
	}
	if snapshot.Status != StatusDown {
		t.Fatalf("expected overall status down, got %#v", snapshot)
	}
}

func TestManagedConnectionRecoversAfterDatabaseReturns(t *testing.T) {
	conn := &stubConnection{name: "recovering", driver: "stub", schema: "main", pingErr: errors.New("db down")}
	driver := "manager-recovering"

	Register(driver, func(context.Context, config.ConnectionConfig, Limits) (Connection, error) {
		return conn, nil
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "recovering", Driver: driver},
		},
	}

	manager, err := NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	managed, err := manager.Connection("recovering")
	if err != nil {
		t.Fatalf("manager connection: %v", err)
	}
	if _, err := managed.ListSchemas(context.Background(), "", 5); err == nil {
		t.Fatalf("expected unavailable error while connection is down")
	}

	conn.setPingErr(nil)
	schemas, err := managed.ListSchemas(context.Background(), "", 5)
	if err != nil {
		t.Fatalf("list schemas after recovery: %v", err)
	}
	if len(schemas) != 1 || schemas[0].Name != "main" {
		t.Fatalf("unexpected schemas after recovery: %#v", schemas)
	}

	info := manager.ConnectionInfos()[0]
	if !info.Available || info.Status != StatusUp {
		t.Fatalf("expected recovered connection up, got %#v", info)
	}
}
