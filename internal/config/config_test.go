package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestValidateAssignsDefaultConnectionName(t *testing.T) {
	t.Parallel()

	cfg := &Config{
		Server: ServerConfig{},
		Limits: LimitsConfig{},
		Connections: []ConnectionConfig{{
			Driver: "oracle",
			Oracle: &OracleConfig{
				JDBCURL:  "jdbc:oracle:thin:@localhost:1521:XE",
				Username: "readuser",
				Password: "readuser",
			},
		}},
	}

	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}
	if cfg.Connections[0].Name != "oracle" {
		t.Fatalf("expected default connection name oracle, got %q", cfg.Connections[0].Name)
	}
}

func TestValidateAssignsDefaultPostgresConnectionName(t *testing.T) {
	t.Parallel()

	cfg := &Config{
		Server: ServerConfig{},
		Limits: LimitsConfig{},
		Connections: []ConnectionConfig{{
			Driver: "postgres",
			Postgres: &PostgresConfig{
				DSN: "postgres://reader:reader@localhost:5432/app?sslmode=disable",
			},
		}},
	}

	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}
	if cfg.Connections[0].Name != "postgres" {
		t.Fatalf("expected default connection name postgres, got %q", cfg.Connections[0].Name)
	}
}

func TestValidateAssignsDefaultSQLiteConnectionName(t *testing.T) {
	t.Parallel()

	cfg := &Config{
		Server: ServerConfig{},
		Limits: LimitsConfig{},
		Connections: []ConnectionConfig{{
			Driver: "sqlite",
			SQLite: &SQLiteConfig{
				DSN: "file:app.db?mode=ro",
			},
		}},
	}

	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}
	if cfg.Connections[0].Name != "sqlite" {
		t.Fatalf("expected default connection name sqlite, got %q", cfg.Connections[0].Name)
	}
}

func TestLoadReadsDebugFromEnv(t *testing.T) {
	t.Setenv("DBMCP_DEBUG", "true")
	t.Setenv("DBMCP_POSTGRES_DSN", "postgres://reader:reader@localhost:5432/app?sslmode=disable")

	cfg, err := Load("")
	if err != nil {
		t.Fatalf("load config: %v", err)
	}
	if !cfg.Server.Debug {
		t.Fatal("expected debug to be enabled from env")
	}
}

func TestLoadMergesFileConnectionsWithEnvConnections(t *testing.T) {
	t.Setenv("DBMCP_POSTGRES_DSN", "postgres://reader:reader@localhost:5432/app?sslmode=disable")
	t.Setenv("DBMCP_POSTGRES_SCHEMA", "research")

	configPath := filepath.Join(t.TempDir(), "dbmcp.json")
	if err := os.WriteFile(configPath, []byte(`{
  "connections": [
    {
      "name": "oracle",
      "driver": "oracle",
      "oracle": {
        "jdbcUrl": "jdbc:oracle:thin:@localhost:1521:XE",
        "username": "readuser",
        "password": "readuser"
      }
    }
  ]
}`), 0o600); err != nil {
		t.Fatalf("write config: %v", err)
	}

	cfg, err := Load(configPath)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}
	if len(cfg.Connections) != 2 {
		t.Fatalf("expected 2 merged connections, got %d", len(cfg.Connections))
	}
	if cfg.Connections[1].Driver != "postgres" {
		t.Fatalf("expected appended postgres connection, got %q", cfg.Connections[1].Driver)
	}
	if cfg.Connections[1].Postgres == nil || cfg.Connections[1].Postgres.Schema != "research" {
		t.Fatalf("expected postgres schema from env, got %#v", cfg.Connections[1].Postgres)
	}
}

func TestLoadEnvConnectionOverridesFileConnectionWithSameName(t *testing.T) {
	t.Setenv("DBMCP_POSTGRES_NAME", "postgres")
	t.Setenv("DBMCP_POSTGRES_DSN", "postgres://reader:newpass@db.example.com:5432/app?sslmode=disable")
	t.Setenv("DBMCP_POSTGRES_SCHEMA", "research")

	configPath := filepath.Join(t.TempDir(), "dbmcp.json")
	if err := os.WriteFile(configPath, []byte(`{
  "connections": [
    {
      "name": "postgres",
      "driver": "postgres",
      "postgres": {
        "dsn": "postgres://reader:oldpass@localhost:5432/app?sslmode=disable",
        "schema": "public"
      }
    }
  ]
}`), 0o600); err != nil {
		t.Fatalf("write config: %v", err)
	}

	cfg, err := Load(configPath)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}
	if len(cfg.Connections) != 1 {
		t.Fatalf("expected 1 overridden connection, got %d", len(cfg.Connections))
	}
	if got := cfg.Connections[0].Postgres.DSN; got != "postgres://reader:newpass@db.example.com:5432/app?sslmode=disable" {
		t.Fatalf("unexpected overridden dsn: %q", got)
	}
	if got := cfg.Connections[0].Postgres.Schema; got != "research" {
		t.Fatalf("unexpected overridden schema: %q", got)
	}
}
