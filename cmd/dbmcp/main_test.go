package main

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/config"
	"db/internal/database"
)

type healthTestConnection struct {
	name    string
	driver  string
	schema  string
	pingErr error
}

func (s *healthTestConnection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{Name: s.name, Driver: s.driver, DefaultSchema: s.schema}
}

func (s *healthTestConnection) Ping(context.Context) error {
	return s.pingErr
}

func (s *healthTestConnection) ListSchemas(context.Context, string, int) ([]database.SchemaInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) ListTables(context.Context, string, string, int) ([]database.TableInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) SearchColumns(context.Context, string, string, int) ([]database.ColumnMatchInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) ListForeignKeys(context.Context, string, string, int) ([]database.ForeignKeyInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) ListIndexes(context.Context, string, string, int) ([]database.IndexInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) GetTableComments(context.Context, string, string) (*database.TableCommentInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) DescribeTable(context.Context, string, string) ([]database.ColumnInfo, error) {
	return nil, nil
}

func (s *healthTestConnection) ExecuteQuery(context.Context, string, int) (*database.QueryResult, error) {
	return nil, nil
}

func (s *healthTestConnection) ExplainQuery(context.Context, string) (*database.ExplainResult, error) {
	return nil, nil
}

func (s *healthTestConnection) Close() error {
	return nil
}

func TestHealthzReturnsLivenessOnly(t *testing.T) {
	t.Parallel()

	handler := buildHTTPHandler(&config.Config{
		Server: config.ServerConfig{HTTPPath: "/mcp"},
	}, nilManager(t), mcp.NewServer(&mcp.Implementation{Name: "test", Version: "v0.0.1"}, nil))

	req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)

	if res.Code != http.StatusOK || res.Body.String() != "ok" {
		t.Fatalf("unexpected healthz response: code=%d body=%q", res.Code, res.Body.String())
	}
}

func TestReadyzReturnsUpAndDegradedAndDown(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name        string
		connections []database.Connection
		wantCode    int
		wantStatus  string
	}{
		{
			name: "up",
			connections: []database.Connection{
				&healthTestConnection{name: "primary", driver: "stub", schema: "main"},
			},
			wantCode:   http.StatusOK,
			wantStatus: database.StatusUp,
		},
		{
			name: "degraded",
			connections: []database.Connection{
				&healthTestConnection{name: "primary", driver: "stub", schema: "main"},
				&healthTestConnection{name: "secondary", driver: "stub", schema: "main", pingErr: errors.New("db down")},
			},
			wantCode:   http.StatusOK,
			wantStatus: database.StatusDegraded,
		},
		{
			name: "down",
			connections: []database.Connection{
				&healthTestConnection{name: "primary", driver: "stub", schema: "main", pingErr: errors.New("db down")},
			},
			wantCode:   http.StatusServiceUnavailable,
			wantStatus: database.StatusDown,
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			manager := newHTTPTestManager(t, tt.name, tt.connections...)
			handler := buildHTTPHandler(&config.Config{
				Server: config.ServerConfig{HTTPPath: "/mcp"},
			}, manager, mcp.NewServer(&mcp.Implementation{Name: "test", Version: "v0.0.1"}, nil))

			req := httptest.NewRequest(http.MethodGet, "/readyz", nil)
			res := httptest.NewRecorder()
			handler.ServeHTTP(res, req)

			if res.Code != tt.wantCode {
				t.Fatalf("unexpected readyz status code: got=%d want=%d body=%q", res.Code, tt.wantCode, res.Body.String())
			}

			var body database.HealthSnapshot
			if err := json.Unmarshal(res.Body.Bytes(), &body); err != nil {
				t.Fatalf("decode readyz body: %v", err)
			}
			if body.Status != tt.wantStatus {
				t.Fatalf("unexpected readyz status: got=%q want=%q", body.Status, tt.wantStatus)
			}
		})
	}
}

func nilManager(t *testing.T) *database.Manager {
	t.Helper()
	manager, err := newHTTPTestManagerWithConfig("nil-manager")
	if err != nil {
		t.Fatalf("new empty manager: %v", err)
	}
	return manager
}

func newHTTPTestManager(t *testing.T, suffix string, connections ...database.Connection) *database.Manager {
	t.Helper()
	manager, err := newHTTPTestManagerWithConfig(suffix, connections...)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	t.Cleanup(func() {
		_ = manager.Close()
	})
	return manager
}

func newHTTPTestManagerWithConfig(suffix string, connections ...database.Connection) (*database.Manager, error) {
	if len(connections) == 0 {
		connections = []database.Connection{
			&healthTestConnection{name: "primary", driver: "stub", schema: "main"},
		}
	}

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
	}
	for i, conn := range connections {
		driver := "cmd-health-" + suffix + "-" + conn.Info().Name
		database.Register(driver, func(conn database.Connection) database.OpenFunc {
			return func(context.Context, config.ConnectionConfig, database.Limits) (database.Connection, error) {
				return conn, nil
			}
		}(conn))
		cfg.Connections = append(cfg.Connections, config.ConnectionConfig{
			Name:   conn.Info().Name,
			Driver: driver,
		})
		_ = i
	}
	return database.NewManager(context.Background(), cfg)
}
