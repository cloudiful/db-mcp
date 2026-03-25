package postgres

import (
	"context"
	"database/sql"
	"fmt"
	"strings"
	"time"

	"db/internal/config"
	"db/internal/database"

	_ "github.com/jackc/pgx/v5/stdlib"
)

func init() {
	database.Register("postgres", openConnection)
}

type connection struct {
	name          string
	defaultSchema string
	db            *sql.DB
	limits        database.Limits
}

func openConnection(ctx context.Context, cfg config.ConnectionConfig, limits database.Limits) (database.Connection, error) {
	db, err := sql.Open("pgx", cfg.Postgres.DSN)
	if err != nil {
		return nil, err
	}
	db.SetConnMaxLifetime(5 * time.Minute)
	db.SetMaxIdleConns(2)
	db.SetMaxOpenConns(4)

	pingCtx, cancel := limits.WithTimeout(ctx)
	defer cancel()
	if err := db.PingContext(pingCtx); err != nil {
		_ = db.Close()
		return nil, err
	}

	defaultSchema, err := fetchDefaultSchema(pingCtx, db)
	if err != nil {
		_ = db.Close()
		return nil, err
	}

	return &connection{
		name:          cfg.Name,
		defaultSchema: defaultSchema,
		db:            db,
		limits:        limits,
	}, nil
}

func fetchDefaultSchema(ctx context.Context, db *sql.DB) (string, error) {
	var schema string
	if err := db.QueryRowContext(ctx, `SELECT current_schema()`).Scan(&schema); err != nil {
		return "", fmt.Errorf("fetch postgres default schema: %w", err)
	}
	if schema == "" {
		schema = "public"
	}
	return schema, nil
}

func (c *connection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{
		Name:          c.name,
		Driver:        "postgres",
		DefaultSchema: c.defaultSchema,
	}
}

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	rows, cancel, err := c.query(ctx, `
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name ILIKE COALESCE(NULLIF($1, ''), schema_name)
ORDER BY schema_name
LIMIT $2`, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres schemas: %w", err)
	}
	defer cancel()
	defer rows.Close()

	var results []database.SchemaInfo
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, fmt.Errorf("scan postgres schema row: %w", err)
		}
		results = append(results, database.SchemaInfo{Name: name})
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres schemas: %w", err)
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cancel, err := c.query(ctx, `
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema = COALESCE(NULLIF($1, ''), table_schema)
  AND table_type IN ('BASE TABLE', 'VIEW')
  AND table_name ILIKE COALESCE(NULLIF($2, ''), table_name)
ORDER BY table_schema, table_type, table_name
LIMIT $3`, exactFilter(schema), likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres tables for schema %q: %w", schema, err)
	}
	defer cancel()
	defer rows.Close()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan postgres table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres tables: %w", err)
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

	rows, cancel, err := c.query(ctx, `
SELECT
  ordinal_position,
  column_name,
  data_type,
  character_maximum_length,
  numeric_precision,
  numeric_scale,
  is_nullable
FROM information_schema.columns
WHERE table_schema = $1
  AND table_name = $2
ORDER BY ordinal_position`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("describe postgres table %s.%s: %w", schema, table, err)
	}
	defer cancel()
	defer rows.Close()

	var results []database.ColumnInfo
	for rows.Next() {
		var (
			item      database.ColumnInfo
			length    sql.NullInt64
			precision sql.NullInt64
			scale     sql.NullInt64
			nullable  string
		)
		if err := rows.Scan(&item.Position, &item.Name, &item.DataType, &length, &precision, &scale, &nullable); err != nil {
			return nil, fmt.Errorf("scan postgres column row: %w", err)
		}
		item.Nullable = strings.EqualFold(nullable, "YES")
		item.Length = toPtr(length)
		item.Precision = toPtr(precision)
		item.Scale = toPtr(scale)
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres columns: %w", err)
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
		return nil, fmt.Errorf("execute postgres query: %w", err)
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

func exactFilter(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return value
}

func likeFilter(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return "%" + value + "%"
}

func toPtr(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	v := value.Int64
	return &v
}
