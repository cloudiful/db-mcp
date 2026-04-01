package config

import "testing"

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
