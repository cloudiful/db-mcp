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
	db, err := sql.Open(driverName, cfg.SQLite.DSN)
	if err != nil {
		return nil, err
	}
	db.SetConnMaxLifetime(5 * time.Minute)
	db.SetMaxIdleConns(1)
	db.SetMaxOpenConns(1)

	pingCtx, cancel := limits.WithTimeout(ctx)
	defer cancel()
	if err := db.PingContext(pingCtx); err != nil {
		_ = db.Close()
		return nil, err
	}

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

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	names, err := c.schemaNames(ctx)
	if err != nil {
		return nil, err
	}
	filter = strings.TrimSpace(strings.ToLower(filter))

	results := make([]database.SchemaInfo, 0, len(names))
	for _, name := range names {
		if filter != "" && !strings.Contains(strings.ToLower(name), filter) {
			continue
		}
		results = append(results, database.SchemaInfo{Name: name})
		if len(results) >= limit {
			break
		}
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	query := fmt.Sprintf(`
SELECT
  %s AS table_schema,
  name AS table_name,
  CASE type WHEN 'table' THEN 'BASE TABLE' WHEN 'view' THEN 'VIEW' ELSE UPPER(type) END AS table_type
FROM %s.sqlite_schema
WHERE type IN ('table', 'view')
  AND name NOT LIKE 'sqlite_%%'
  AND name LIKE COALESCE(NULLIF(?1, ''), name) COLLATE NOCASE
ORDER BY table_name
LIMIT ?2`, quoteSQLiteString(schema), quoteSQLiteIdentifier(schema))

	rows, cancel, err := c.query(ctx, query, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list sqlite tables for schema %q: %w", schema, err)
	}
	defer cancel()
	defer rows.Close()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan sqlite table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite tables: %w", err)
	}
	return results, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	query := fmt.Sprintf(`
PRAGMA %s.table_info(%s)`,
		quoteSQLiteIdentifier(schema),
		quoteSQLiteString(table),
	)

	rows, cancel, err := c.query(ctx, query)
	if err != nil {
		return nil, fmt.Errorf("describe sqlite table %s.%s: %w", schema, table, err)
	}
	defer cancel()
	defer rows.Close()

	var results []database.ColumnInfo
	for rows.Next() {
		var (
			cid        int
			item       database.ColumnInfo
			notNull    int
			defaultVal any
			pk         int
		)
		if err := rows.Scan(&cid, &item.Name, &item.DataType, &notNull, &defaultVal, &pk); err != nil {
			return nil, fmt.Errorf("scan sqlite column row: %w", err)
		}
		_ = defaultVal
		_ = pk
		item.Position = cid + 1
		item.Nullable = notNull == 0
		if length, precision, scale := parseSQLiteType(item.DataType); length != nil || precision != nil || scale != nil {
			item.Length = length
			item.Precision = precision
			item.Scale = scale
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite columns: %w", err)
	}
	if len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	return results, nil
}

func (c *connection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*database.QueryResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	appliedMaxRows := c.limits.NormalizeMaxRows(maxRows)
	startedAt := time.Now()
	rows, cancel, err := c.query(ctx, normalized)
	if err != nil {
		return nil, fmt.Errorf("execute sqlite query: %w", err)
	}
	defer cancel()
	return database.CollectRows(rows, appliedMaxRows, c.limits, startedAt)
}

func (c *connection) Close() error {
	return c.db.Close()
}

func (c *connection) query(ctx context.Context, query string, args ...any) (*sql.Rows, context.CancelFunc, error) {
	queryCtx, cancel := c.limits.WithTimeout(ctx)
	rows, err := c.db.QueryContext(queryCtx, query, args...)
	if err != nil {
		cancel()
		return nil, nil, err
	}
	return rows, cancel, nil
}

func (c *connection) schemaNames(ctx context.Context) ([]string, error) {
	rows, cancel, err := c.query(ctx, `PRAGMA database_list`)
	if err != nil {
		return nil, fmt.Errorf("list sqlite schemas: %w", err)
	}
	defer cancel()
	defer rows.Close()

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
