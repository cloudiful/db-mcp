package config

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"
)

var ErrInvalidTransport = errors.New("invalid transport")

type TransportMode string

const (
	TransportStdio TransportMode = "stdio"
	TransportHTTP  TransportMode = "http"
	TransportBoth  TransportMode = "both"
)

type Config struct {
	Server      ServerConfig       `json:"server"`
	Limits      LimitsConfig       `json:"limits"`
	Connections []ConnectionConfig `json:"connections"`
}

type ServerConfig struct {
	Name      string        `json:"name"`
	Version   string        `json:"version"`
	Transport TransportMode `json:"transport"`
	HTTPAddr  string        `json:"httpAddr"`
	HTTPPath  string        `json:"httpPath"`
}

type LimitsConfig struct {
	DefaultMaxRows int    `json:"defaultMaxRows"`
	HardMaxRows    int    `json:"hardMaxRows"`
	MaxColumns     int    `json:"maxColumns"`
	MaxCellChars   int    `json:"maxCellChars"`
	MaxTotalChars  int    `json:"maxTotalChars"`
	QueryTimeout   string `json:"queryTimeout"`
}

type ConnectionConfig struct {
	Name     string          `json:"name"`
	Driver   string          `json:"driver"`
	Oracle   *OracleConfig   `json:"oracle,omitempty"`
	Postgres *PostgresConfig `json:"postgres,omitempty"`
	SQLite   *SQLiteConfig   `json:"sqlite,omitempty"`
}

type OracleConfig struct {
	JDBCURL  string `json:"jdbcUrl"`
	Username string `json:"username"`
	Password string `json:"password"`
}

type PostgresConfig struct {
	DSN string `json:"dsn"`
}

type SQLiteConfig struct {
	DSN string `json:"dsn"`
}

func Load(path string) (*Config, error) {
	cfg := defaultConfig()

	if path != "" {
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, fmt.Errorf("read config: %w", err)
		}
		if err := json.Unmarshal(data, &cfg); err != nil {
			return nil, fmt.Errorf("parse config: %w", err)
		}
	} else {
		loadEnv(cfg)
	}

	if len(cfg.Connections) == 0 {
		loadEnv(cfg)
	}
	return cfg, nil
}

func (c *Config) Validate() error {
	if c.Server.Name == "" {
		c.Server.Name = "db-mcp"
	}
	if c.Server.Version == "" {
		c.Server.Version = "0.1.0"
	}
	if c.Server.Transport == "" {
		c.Server.Transport = TransportStdio
	}
	switch c.Server.Transport {
	case TransportStdio, TransportHTTP, TransportBoth:
	default:
		return ErrInvalidTransport
	}

	if c.Server.HTTPAddr == "" {
		c.Server.HTTPAddr = ":8080"
	}
	if c.Server.HTTPPath == "" {
		c.Server.HTTPPath = "/mcp"
	}

	if c.Limits.DefaultMaxRows <= 0 {
		c.Limits.DefaultMaxRows = 50
	}
	if c.Limits.HardMaxRows <= 0 {
		c.Limits.HardMaxRows = 200
	}
	if c.Limits.DefaultMaxRows > c.Limits.HardMaxRows {
		c.Limits.DefaultMaxRows = c.Limits.HardMaxRows
	}
	if c.Limits.MaxColumns <= 0 {
		c.Limits.MaxColumns = 40
	}
	if c.Limits.MaxCellChars <= 0 {
		c.Limits.MaxCellChars = 512
	}
	if c.Limits.MaxTotalChars <= 0 {
		c.Limits.MaxTotalChars = 12000
	}
	if c.Limits.QueryTimeout == "" {
		c.Limits.QueryTimeout = "15s"
	}
	if _, err := time.ParseDuration(c.Limits.QueryTimeout); err != nil {
		return fmt.Errorf("invalid queryTimeout: %w", err)
	}

	if len(c.Connections) == 0 {
		return errors.New("at least one connection must be configured")
	}
	seen := map[string]struct{}{}
	for i := range c.Connections {
		conn := &c.Connections[i]
		if conn.Name == "" {
			conn.Name = defaultConnectionName(conn.Driver, i)
		}
		if _, ok := seen[conn.Name]; ok {
			return fmt.Errorf("duplicate connection name %q", conn.Name)
		}
		seen[conn.Name] = struct{}{}
		switch strings.ToLower(conn.Driver) {
		case "oracle":
			if conn.Oracle == nil {
				return fmt.Errorf("connections[%d].oracle is required", i)
			}
			if conn.Oracle.JDBCURL == "" || conn.Oracle.Username == "" || conn.Oracle.Password == "" {
				return fmt.Errorf("connections[%d].oracle requires jdbcUrl, username, and password", i)
			}
		case "postgres":
			if conn.Postgres == nil {
				return fmt.Errorf("connections[%d].postgres is required", i)
			}
			if conn.Postgres.DSN == "" {
				return fmt.Errorf("connections[%d].postgres requires dsn", i)
			}
		case "sqlite":
			if conn.SQLite == nil {
				return fmt.Errorf("connections[%d].sqlite is required", i)
			}
			if conn.SQLite.DSN == "" {
				return fmt.Errorf("connections[%d].sqlite requires dsn", i)
			}
		default:
			return fmt.Errorf("unsupported driver %q", conn.Driver)
		}
		conn.Driver = strings.ToLower(conn.Driver)
	}
	return nil
}

func (c *Config) QueryTimeout() time.Duration {
	d, _ := time.ParseDuration(c.Limits.QueryTimeout)
	return d
}

func defaultConfig() *Config {
	return &Config{
		Server: ServerConfig{
			Name:      "db-mcp",
			Version:   "0.1.0",
			Transport: TransportStdio,
			HTTPAddr:  ":8080",
			HTTPPath:  "/mcp",
		},
		Limits: LimitsConfig{
			DefaultMaxRows: 50,
			HardMaxRows:    200,
			MaxColumns:     40,
			MaxCellChars:   512,
			MaxTotalChars:  12000,
			QueryTimeout:   "15s",
		},
	}
}

func loadEnv(cfg *Config) {
	if value := os.Getenv("DBMCP_NAME"); value != "" {
		cfg.Server.Name = value
	}
	if value := os.Getenv("DBMCP_VERSION"); value != "" {
		cfg.Server.Version = value
	}
	if value := os.Getenv("DBMCP_TRANSPORT"); value != "" {
		cfg.Server.Transport = TransportMode(value)
	}
	if value := os.Getenv("DBMCP_HTTP_ADDR"); value != "" {
		cfg.Server.HTTPAddr = value
	}
	if value := os.Getenv("DBMCP_HTTP_PATH"); value != "" {
		cfg.Server.HTTPPath = value
	}
	if value := os.Getenv("DBMCP_QUERY_TIMEOUT"); value != "" {
		cfg.Limits.QueryTimeout = value
	}
	var connections []ConnectionConfig
	if value := os.Getenv("DBMCP_ORACLE_JDBC_URL"); value != "" {
		connections = append(connections, ConnectionConfig{
			Name:   envOr("DBMCP_ORACLE_NAME", "oracle"),
			Driver: "oracle",
			Oracle: &OracleConfig{
				JDBCURL:  value,
				Username: os.Getenv("DBMCP_ORACLE_USERNAME"),
				Password: os.Getenv("DBMCP_ORACLE_PASSWORD"),
			},
		})
	}
	if value := envOr("DBMCP_POSTGRES_DSN", os.Getenv("DATABASE_URL")); value != "" {
		connections = append(connections, ConnectionConfig{
			Name:   envOr("DBMCP_POSTGRES_NAME", "postgres"),
			Driver: "postgres",
			Postgres: &PostgresConfig{
				DSN: value,
			},
		})
	}
	if value := envOr("DBMCP_SQLITE_DSN", os.Getenv("SQLITE_URL")); value != "" {
		connections = append(connections, ConnectionConfig{
			Name:   envOr("DBMCP_SQLITE_NAME", "sqlite"),
			Driver: "sqlite",
			SQLite: &SQLiteConfig{
				DSN: value,
			},
		})
	}
	if len(connections) > 0 {
		cfg.Connections = connections
	}
}

func envOr(key, fallback string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return fallback
}

func defaultConnectionName(driver string, index int) string {
	driver = strings.ToLower(strings.TrimSpace(driver))
	if driver == "" {
		return fmt.Sprintf("connection-%d", index+1)
	}
	if index == 0 {
		return driver
	}
	return fmt.Sprintf("%s-%d", driver, index+1)
}
