package oracle

import (
	"context"
	"database/sql"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"time"

	"db/internal/config"
	"db/internal/database"

	go_ora "github.com/sijms/go-ora/v2"
)

func init() {
	database.Register("oracle", openConnection)
}

type connection struct {
	name          string
	defaultSchema string
	db            *sql.DB
	limits        database.Limits
}

func openConnection(ctx context.Context, cfg config.ConnectionConfig, limits database.Limits) (database.Connection, error) {
	_ = ctx
	dsn, err := buildOracleDSN(cfg.Oracle.Username, cfg.Oracle.Password, cfg.Oracle.JDBCURL, limits.QueryTimeout)
	if err != nil {
		return nil, err
	}

	db, err := sql.Open("oracle", dsn)
	if err != nil {
		return nil, err
	}
	db.SetConnMaxLifetime(5 * time.Minute)
	db.SetMaxIdleConns(2)
	db.SetMaxOpenConns(4)

	return &connection{
		name:          cfg.Name,
		defaultSchema: strings.ToUpper(cfg.Oracle.Username),
		db:            db,
		limits:        limits,
	}, nil
}

func buildOracleDSN(username, password, jdbcURL string, timeout time.Duration) (string, error) {
	options := map[string]string{
		"CONNECTION TIMEOUT": strconv.Itoa(maxInt(1, int(timeout.Seconds()))),
	}
	host, port, service, err := parseJDBCURL(jdbcURL)
	if err != nil {
		return "", err
	}
	return go_ora.BuildUrl(host, port, service, username, password, options), nil
}

func (c *connection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{
		Name:          c.name,
		Driver:        "oracle",
		DefaultSchema: c.defaultSchema,
	}
}

func (c *connection) Ping(ctx context.Context) error {
	pingCtx, cancel := c.limits.WithTimeout(ctx)
	defer cancel()
	return c.db.PingContext(pingCtx)
}

func (c *connection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*database.QueryResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	appliedMaxRows := c.limits.NormalizeMaxRows(maxRows)
	startedAt := time.Now()
	rows, cleanup, err := c.query(ctx, normalized)
	if err != nil {
		return nil, fmt.Errorf("execute oracle query: %w", err)
	}
	result, err := database.CollectRows(rows, appliedMaxRows, c.limits, startedAt)
	if cleanupErr := cleanup(); err == nil && cleanupErr != nil {
		err = cleanupErr
	}
	return result, err
}

func (c *connection) ExplainQuery(context.Context, string) (*database.ExplainResult, error) {
	return nil, fmt.Errorf("oracle explain is not supported because EXPLAIN PLAN requires writable plan tables; this server keeps Oracle access strictly read-only")
}

func (c *connection) Close() error {
	return c.db.Close()
}

func (c *connection) query(ctx context.Context, query string, args ...any) (*sql.Rows, func() error, error) {
	queryCtx, cancel := c.limits.WithTimeout(ctx)
	tx, err := c.db.BeginTx(queryCtx, oracleQueryTxOptions())
	if err != nil {
		cancel()
		return nil, nil, err
	}
	if _, err := tx.ExecContext(queryCtx, "SET TRANSACTION READ ONLY"); err != nil {
		_ = tx.Rollback()
		cancel()
		return nil, nil, err
	}
	rows, err := tx.QueryContext(queryCtx, query, args...)
	if err != nil {
		_ = tx.Rollback()
		cancel()
		return nil, nil, err
	}
	return rows, func() error {
		var firstErr error
		if err := rows.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
		if err := tx.Rollback(); err != nil && err != sql.ErrTxDone && firstErr == nil {
			firstErr = err
		}
		cancel()
		return firstErr
	}, nil
}

func oracleQueryTxOptions() *sql.TxOptions {
	// go-ora rejects TxOptions.ReadOnly, so we enter a default transaction and
	// enforce Oracle's read-only mode with SET TRANSACTION READ ONLY instead.
	return &sql.TxOptions{}
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}

func parseJDBCURL(raw string) (host string, port int, service string, err error) {
	const prefix = "jdbc:oracle:thin:@"
	if !strings.HasPrefix(strings.ToLower(raw), prefix) {
		return "", 0, "", fmt.Errorf("unsupported Oracle JDBC URL %q", raw)
	}

	target := raw[len(prefix):]
	target = strings.TrimSpace(target)
	target = strings.TrimPrefix(target, "//")
	target = strings.TrimPrefix(target, "@")

	if strings.Contains(target, "/") {
		u, parseErr := url.Parse("oracle://" + target)
		if parseErr != nil {
			return "", 0, "", fmt.Errorf("parse JDBC URL: %w", parseErr)
		}
		host = u.Hostname()
		port = 1521
		if parsedPort := u.Port(); parsedPort != "" {
			port, err = strconv.Atoi(parsedPort)
			if err != nil {
				return "", 0, "", fmt.Errorf("parse JDBC port: %w", err)
			}
		}
		service = strings.TrimPrefix(u.Path, "/")
		if host == "" || service == "" {
			return "", 0, "", fmt.Errorf("invalid JDBC URL %q", raw)
		}
		return host, port, service, nil
	}

	parts := strings.Split(target, ":")
	if len(parts) == 3 {
		port, err = strconv.Atoi(parts[1])
		if err != nil {
			return "", 0, "", fmt.Errorf("parse JDBC port: %w", err)
		}
		if parts[0] == "" || parts[2] == "" {
			return "", 0, "", fmt.Errorf("invalid JDBC URL %q", raw)
		}
		return parts[0], port, parts[2], nil
	}

	return "", 0, "", fmt.Errorf("unsupported Oracle JDBC URL shape %q", raw)
}
