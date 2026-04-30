package sqlite

import (
	"context"
	"database/sql"
	"fmt"
	"strconv"
	"strings"
	"time"

	"db/internal/config"
	"db/internal/database"

	sqlite3 "modernc.org/sqlite"
)

const driverName = "sqlite_query_only"

func init() {
	driver := &sqlite3.Driver{}
	driver.RegisterConnectionHook(func(conn sqlite3.ExecQuerierContext, _ string) error {
		_, err := conn.ExecContext(context.Background(), `PRAGMA query_only = ON`, nil)
		return err
	})
	sql.Register(driverName, driver)
	database.Register("sqlite", openConnection)
}

type connection struct {
	name          string
	defaultSchema string
	db            *sql.DB
	limits        database.Limits
}

func openConnection(ctx context.Context, cfg config.ConnectionConfig, limits database.Limits) (database.Connection, error) {
	_ = ctx
	db, err := sql.Open(driverName, cfg.SQLite.DSN)
	if err != nil {
		return nil, err
	}
	db.SetConnMaxLifetime(5 * time.Minute)
	db.SetMaxIdleConns(1)
	db.SetMaxOpenConns(1)

	return &connection{
		name:          cfg.Name,
		defaultSchema: "main",
		db:            db,
		limits:        limits,
	}, nil
}

func (c *connection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{
		Name:          c.name,
		Driver:        "sqlite",
		DefaultSchema: c.defaultSchema,
	}
}

func (c *connection) Ping(ctx context.Context) error {
	pingCtx, cancel := c.limits.WithTimeout(ctx)
	defer cancel()
	return c.db.PingContext(pingCtx)
}

func (c *connection) Close() error {
	return c.db.Close()
}

func (c *connection) query(ctx context.Context, query string, args ...any) (*sql.Rows, func() error, error) {
	queryCtx, cancel := c.limits.WithTimeout(ctx)
	tx, err := c.db.BeginTx(queryCtx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
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

func (c *connection) schemaNames(ctx context.Context) ([]string, error) {
	rows, cleanup, err := c.query(ctx, `PRAGMA database_list`)
	if err != nil {
		return nil, fmt.Errorf("list sqlite schemas: %w", err)
	}
	defer cleanup()

	var names []string
	for rows.Next() {
		var seq int
		var name string
		var file string
		if err := rows.Scan(&seq, &name, &file); err != nil {
			return nil, fmt.Errorf("scan sqlite schema row: %w", err)
		}
		_ = seq
		_ = file
		names = append(names, name)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite schemas: %w", err)
	}
	return names, nil
}

func (c *connection) validateSchema(ctx context.Context, schema string) (string, error) {
	schema = strings.TrimSpace(schema)
	names, err := c.schemaNames(ctx)
	if err != nil {
		return "", err
	}
	for _, name := range names {
		if name == schema {
			return schema, nil
		}
	}
	return "", fmt.Errorf("sqlite schema %q not found", schema)
}

func (c *connection) listTableNames(ctx context.Context, schema, table string) ([]string, error) {
	query := fmt.Sprintf(`
SELECT name
FROM %s.sqlite_schema
WHERE type IN ('table', 'view')
  AND name NOT LIKE 'sqlite_%%'
  AND name = COALESCE(NULLIF(?1, ''), name)
ORDER BY name`, quoteSQLiteIdentifier(schema))

	rows, cleanup, err := c.query(ctx, query, strings.TrimSpace(table))
	if err != nil {
		return nil, fmt.Errorf("list sqlite table names for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []string
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, fmt.Errorf("scan sqlite table name: %w", err)
		}
		results = append(results, name)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite table names: %w", err)
	}
	if strings.TrimSpace(table) != "" && len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	return results, nil
}

func quoteSQLiteIdentifier(value string) string {
	return `"` + strings.ReplaceAll(value, `"`, `""`) + `"`
}

func quoteSQLiteString(value string) string {
	return `'` + strings.ReplaceAll(value, `'`, `''`) + `'`
}

func likeFilter(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return "%" + value + "%"
}

func parseSQLiteType(value string) (length, precision, scale *int64) {
	value = strings.TrimSpace(strings.ToUpper(value))
	if value == "" {
		return nil, nil, nil
	}
	start := strings.Index(value, "(")
	end := strings.LastIndex(value, ")")
	if start == -1 || end <= start+1 {
		return nil, nil, nil
	}
	args := strings.Split(value[start+1:end], ",")
	switch len(args) {
	case 1:
		if parsed, err := strconv.ParseInt(strings.TrimSpace(args[0]), 10, 64); err == nil {
			return &parsed, nil, nil
		}
	case 2:
		p, errP := strconv.ParseInt(strings.TrimSpace(args[0]), 10, 64)
		s, errS := strconv.ParseInt(strings.TrimSpace(args[1]), 10, 64)
		if errP == nil && errS == nil {
			return nil, &p, &s
		}
	}
	return nil, nil, nil
}
