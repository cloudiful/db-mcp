package database

import (
	"context"
	"errors"
	"testing"

	"db/internal/config"
)

type stubConnection struct {
	closed bool
}

func (s *stubConnection) Info() ConnectionInfo {
	return ConnectionInfo{Name: "stub", Driver: "stub", DefaultSchema: "main"}
}

func (s *stubConnection) ListSchemas(context.Context, string, int) ([]SchemaInfo, error) {
	return nil, nil
}

func (s *stubConnection) ListTables(context.Context, string, string, int) ([]TableInfo, error) {
	return nil, nil
}

func (s *stubConnection) DescribeTable(context.Context, string, string) ([]ColumnInfo, error) {
	return nil, nil
}

func (s *stubConnection) ExecuteQuery(context.Context, string, int) (*QueryResult, error) {
	return nil, nil
}

func (s *stubConnection) Close() error {
	s.closed = true
	return nil
}

func TestNewManagerClosesOpenedConnectionsOnFailure(t *testing.T) {
	primary := &stubConnection{}
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
	if !primary.closed {
		t.Fatalf("expected opened connections to be closed on failure")
	}
}
