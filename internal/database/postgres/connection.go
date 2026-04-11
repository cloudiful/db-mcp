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
	if strings.TrimSpace(cfg.Postgres.Schema) != "" {
		defaultSchema = strings.TrimSpace(cfg.Postgres.Schema)
		if err := ensureSchemaVisible(pingCtx, db, defaultSchema); err != nil {
			_ = db.Close()
			return nil, err
		}
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

func ensureSchemaVisible(ctx context.Context, db *sql.DB, schema string) error {
	var exists bool
	if err := db.QueryRowContext(ctx, `
SELECT EXISTS (
  SELECT 1
  FROM information_schema.schemata
  WHERE schema_name = $1
)`, schema).Scan(&exists); err != nil {
		return fmt.Errorf("check postgres schema %q: %w", schema, err)
	}
	if !exists {
		return fmt.Errorf("postgres schema %q not found or not visible", schema)
	}
	return nil
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
	rows, cleanup, err := c.query(ctx, `
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name ILIKE COALESCE(NULLIF($1, ''), schema_name)
ORDER BY schema_name
LIMIT $2`, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres schemas: %w", err)
	}
	defer cleanup()

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

	rows, cleanup, err := c.query(ctx, `
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
	defer cleanup()

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

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
SELECT table_schema, table_name, column_name, data_type, ordinal_position
FROM information_schema.columns
WHERE table_schema = COALESCE(NULLIF($1, ''), table_schema)
  AND column_name ILIKE COALESCE(NULLIF($2, ''), column_name)
ORDER BY table_schema, table_name, ordinal_position
LIMIT $3`, exactFilter(schema), likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("search postgres columns for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.ColumnMatchInfo
	for rows.Next() {
		var item database.ColumnMatchInfo
		if err := rows.Scan(&item.Schema, &item.Table, &item.Column, &item.DataType, &item.Position); err != nil {
			return nil, fmt.Errorf("scan postgres column match row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres column matches: %w", err)
	}
	return results, nil
}

func (c *connection) ListForeignKeys(ctx context.Context, schema, table string, limit int) ([]database.ForeignKeyInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  fk.constraint_name,
  fk.table_schema,
  fk.table_name,
  fk.column_name,
  fk.ordinal_position,
  pk.table_schema AS referenced_table_schema,
  pk.table_name AS referenced_table_name,
  pk.column_name AS referenced_column_name,
  rc.update_rule,
  rc.delete_rule
FROM information_schema.key_column_usage fk
JOIN information_schema.referential_constraints rc
  ON rc.constraint_schema = fk.constraint_schema
 AND rc.constraint_name = fk.constraint_name
JOIN information_schema.key_column_usage pk
  ON pk.constraint_schema = rc.unique_constraint_schema
 AND pk.constraint_name = rc.unique_constraint_name
 AND pk.ordinal_position = fk.position_in_unique_constraint
WHERE fk.table_schema = COALESCE(NULLIF($1, ''), fk.table_schema)
  AND fk.table_name = COALESCE(NULLIF($2, ''), fk.table_name)
ORDER BY fk.table_schema, fk.table_name, fk.constraint_name, fk.ordinal_position
LIMIT $3`, exactFilter(schema), exactFilter(table), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres foreign keys for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.ForeignKeyInfo
	for rows.Next() {
		var item database.ForeignKeyInfo
		if err := rows.Scan(
			&item.ConstraintName,
			&item.Schema,
			&item.Table,
			&item.Column,
			&item.Position,
			&item.ReferencedSchema,
			&item.ReferencedTable,
			&item.ReferencedColumn,
			&item.UpdateRule,
			&item.DeleteRule,
		); err != nil {
			return nil, fmt.Errorf("scan postgres foreign key row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres foreign keys: %w", err)
	}
	return results, nil
}

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  ns.nspname,
  tbl.relname,
  idx.relname,
  am.amname,
  i.indisunique,
  i.indisprimary,
  (i.indpred IS NOT NULL) AS is_partial,
  COALESCE(string_agg(att.attname, ',' ORDER BY ord.ordinality), '') AS columns_csv,
  pg_get_indexdef(i.indexrelid)
FROM pg_class tbl
JOIN pg_namespace ns ON ns.oid = tbl.relnamespace
JOIN pg_index i ON i.indrelid = tbl.oid
JOIN pg_class idx ON idx.oid = i.indexrelid
JOIN pg_am am ON am.oid = idx.relam
LEFT JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS ord(attnum, ordinality) ON TRUE
LEFT JOIN pg_attribute att ON att.attrelid = tbl.oid AND att.attnum = ord.attnum
WHERE ns.nspname = COALESCE(NULLIF($1, ''), ns.nspname)
  AND tbl.relname = COALESCE(NULLIF($2, ''), tbl.relname)
GROUP BY ns.nspname, tbl.relname, idx.relname, am.amname, i.indisunique, i.indisprimary, is_partial, i.indexrelid
ORDER BY ns.nspname, tbl.relname, idx.relname
LIMIT $3`, exactFilter(schema), exactFilter(table), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres indexes for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.IndexInfo
	for rows.Next() {
		var (
			item       database.IndexInfo
			columnsCSV string
		)
		if err := rows.Scan(
			&item.Schema,
			&item.Table,
			&item.Name,
			&item.Type,
			&item.Unique,
			&item.Primary,
			&item.Partial,
			&columnsCSV,
			&item.Definition,
		); err != nil {
			return nil, fmt.Errorf("scan postgres index row: %w", err)
		}
		item.Columns = splitCSV(columnsCSV)
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres indexes: %w", err)
	}
	return results, nil
}

func (c *connection) GetTableComments(ctx context.Context, schema, table string) (*database.TableCommentInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
SELECT COALESCE(obj_description(c.oid, 'pg_class'), '')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1
  AND c.relname = $2
  AND c.relkind IN ('r', 'v', 'm', 'p', 'f')`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get postgres table comment for %s.%s: %w", schema, table, err)
	}
	var result database.TableCommentInfo
	result.Schema = schema
	result.Table = table
	if !rows.Next() {
		cleanup()
		if err := rows.Err(); err != nil {
			return nil, fmt.Errorf("iterate postgres table comment: %w", err)
		}
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	if err := rows.Scan(&result.TableComment); err != nil {
		cleanup()
		return nil, fmt.Errorf("scan postgres table comment: %w", err)
	}
	if err := cleanup(); err != nil {
		return nil, err
	}

	rows, cleanup, err = c.query(ctx, `
SELECT
  a.attname,
  COALESCE(col_description(c.oid, a.attnum), '')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid
WHERE n.nspname = $1
  AND c.relname = $2
  AND a.attnum > 0
  AND NOT a.attisdropped
ORDER BY a.attnum`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get postgres column comments for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	for rows.Next() {
		var item database.ColumnCommentInfo
		if err := rows.Scan(&item.Name, &item.Comment); err != nil {
			return nil, fmt.Errorf("scan postgres column comment row: %w", err)
		}
		result.Columns = append(result.Columns, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres column comments: %w", err)
	}
	return &result, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
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
	defer cleanup()

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
	rows, cleanup, err := c.query(ctx, normalized)
	if err != nil {
		return nil, fmt.Errorf("execute postgres query: %w", err)
	}
	result, err := database.CollectRows(rows, appliedMaxRows, c.limits, startedAt)
	if cleanupErr := cleanup(); err == nil && cleanupErr != nil {
		err = cleanupErr
	}
	return result, err
}

func (c *connection) ExplainQuery(ctx context.Context, query string) (*database.ExplainResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	rows, cleanup, err := c.query(ctx, "EXPLAIN (FORMAT TEXT) "+normalized)
	if err != nil {
		return nil, fmt.Errorf("explain postgres query: %w", err)
	}
	defer cleanup()

	var lines []string
	for rows.Next() {
		var line string
		if err := rows.Scan(&line); err != nil {
			return nil, fmt.Errorf("scan postgres explain row: %w", err)
		}
		lines = append(lines, line)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres explain rows: %w", err)
	}
	lines, truncated := database.TruncateLines(lines, c.limits.ExplainMaxLines, c.limits.ExplainMaxTotalChars)
	result := &database.ExplainResult{
		Format:    "text",
		PlanLines: lines,
	}
	if truncated {
		result.Notice = "Explain output was truncated to protect MCP context size."
	}
	return result, nil
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
	if err := setLocalSearchPath(queryCtx, tx, c.defaultSchema); err != nil {
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

func setLocalSearchPath(ctx context.Context, tx *sql.Tx, schema string) error {
	schema = strings.TrimSpace(schema)
	if schema == "" {
		return nil
	}

	searchPath := quoteIdentifier(schema)
	if !strings.EqualFold(schema, "public") {
		searchPath += ", public"
	}
	if _, err := tx.ExecContext(ctx, "SET LOCAL search_path TO "+searchPath); err != nil {
		return fmt.Errorf("set postgres search_path: %w", err)
	}
	return nil
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

func splitCSV(value string) []string {
	if value == "" {
		return nil
	}
	parts := strings.Split(value, ",")
	result := make([]string, 0, len(parts))
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if part != "" {
			result = append(result, part)
		}
	}
	return result
}

func quoteIdentifier(value string) string {
	return `"` + strings.ReplaceAll(value, `"`, `""`) + `"`
}

func toPtr(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	v := value.Int64
	return &v
}
