package mcpserver

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"

	"db/internal/config"
	"db/internal/database"
	_ "db/internal/database/sqlite"
)

func TestDescribeTablePreservesRequestedCaseForNonOracleDrivers(t *testing.T) {
	t.Parallel()

	tempDir := t.TempDir()
	dbPath := filepath.Join(tempDir, "test.db")

	writableDB, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatalf("open writable sqlite db: %v", err)
	}
	defer writableDB.Close()

	if _, err := writableDB.Exec(`CREATE TABLE user_profile (id INTEGER PRIMARY KEY, display_name TEXT)`); err != nil {
		t.Fatalf("create table: %v", err)
	}

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{{
			Name:   "sqlite-test",
			Driver: "sqlite",
			SQLite: &config.SQLiteConfig{
				DSN: "file:" + dbPath + "?mode=ro",
			},
		}},
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}

	manager, err := database.NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.describeTable(context.Background(), nil, describeTableInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Table:      "user_profile",
	})
	if err != nil {
		t.Fatalf("describe table: %v", err)
	}
	if output.Schema != "main" {
		t.Fatalf("expected schema to preserve case, got %q", output.Schema)
	}
	if output.Table != "user_profile" {
		t.Fatalf("expected table to preserve case, got %q", output.Table)
	}
}
