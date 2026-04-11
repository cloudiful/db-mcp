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

	rows, cleanup, err := c.query(ctx, query, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list sqlite tables for schema %q: %w", schema, err)
	}
	defer cleanup()

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

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	tableNames, err := c.listTableNames(ctx, schema, "")
	if err != nil {
		return nil, err
	}

	columnFilter := strings.TrimSpace(strings.ToLower(filter))
	results := make([]database.ColumnMatchInfo, 0, limit)
	for _, tableName := range tableNames {
		columns, err := c.DescribeTable(ctx, schema, tableName)
		if err != nil {
			return nil, fmt.Errorf("describe sqlite table %s.%s for column search: %w", schema, tableName, err)
		}
		for _, column := range columns {
			if columnFilter != "" && !strings.Contains(strings.ToLower(column.Name), columnFilter) {
				continue
			}
			results = append(results, database.ColumnMatchInfo{
				Schema:   schema,
				Table:    tableName,
				Column:   column.Name,
				DataType: column.DataType,
				Position: column.Position,
			})
			if len(results) >= limit {
				return results, nil
			}
		}
	}
	return results, nil
}

func (c *connection) ListForeignKeys(ctx context.Context, schema, table string, limit int) ([]database.ForeignKeyInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	tableNames, err := c.listTableNames(ctx, schema, table)
	if err != nil {
		return nil, err
	}

	results := make([]database.ForeignKeyInfo, 0, limit)
	for _, tableName := range tableNames {
		query := fmt.Sprintf(`PRAGMA %s.foreign_key_list(%s)`,
			quoteSQLiteIdentifier(schema),
			quoteSQLiteString(tableName),
		)
		rows, cleanup, err := c.query(ctx, query)
		if err != nil {
			return nil, fmt.Errorf("list sqlite foreign keys for %s.%s: %w", schema, tableName, err)
		}

		for rows.Next() {
			var (
				id              int
				seq             int
				referencedTable string
				column          string
				referencedCol   sql.NullString
				updateRule      string
				deleteRule      string
				matchRule       string
			)
			if err := rows.Scan(&id, &seq, &referencedTable, &column, &referencedCol, &updateRule, &deleteRule, &matchRule); err != nil {
				cleanup()
				return nil, fmt.Errorf("scan sqlite foreign key row: %w", err)
			}
			_ = matchRule
			item := database.ForeignKeyInfo{
				ConstraintName:   fmt.Sprintf("fk_%s_%d", tableName, id),
				Schema:           schema,
				Table:            tableName,
				Column:           column,
				Position:         seq + 1,
				ReferencedSchema: schema,
				ReferencedTable:  referencedTable,
				UpdateRule:       strings.ToUpper(updateRule),
				DeleteRule:       strings.ToUpper(deleteRule),
			}
			if referencedCol.Valid {
				item.ReferencedColumn = referencedCol.String
			}
			results = append(results, item)
			if len(results) >= limit {
				cleanup()
				return results, nil
			}
		}
		if err := rows.Err(); err != nil {
			cleanup()
			return nil, fmt.Errorf("iterate sqlite foreign keys for %s.%s: %w", schema, tableName, err)
		}
		if err := cleanup(); err != nil {
			return nil, fmt.Errorf("close sqlite foreign key rows for %s.%s: %w", schema, tableName, err)
		}
	}
	return results, nil
}

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	tableNames, err := c.listTableNames(ctx, schema, table)
	if err != nil {
		return nil, err
	}

	results := make([]database.IndexInfo, 0, limit)
	for _, tableName := range tableNames {
		query := fmt.Sprintf(`PRAGMA %s.index_list(%s)`,
			quoteSQLiteIdentifier(schema),
			quoteSQLiteString(tableName),
		)
		rows, cleanup, err := c.query(ctx, query)
		if err != nil {
			return nil, fmt.Errorf("list sqlite indexes for %s.%s: %w", schema, tableName, err)
		}

		type pendingIndex struct {
			name    string
			unique  bool
			origin  string
			partial bool
		}
		var pending []pendingIndex
		for rows.Next() {
			var (
				seq     int
				name    string
				unique  int
				origin  string
				partial int
			)
			if err := rows.Scan(&seq, &name, &unique, &origin, &partial); err != nil {
				cleanup()
				return nil, fmt.Errorf("scan sqlite index list row: %w", err)
			}
			_ = seq
			pending = append(pending, pendingIndex{
				name:    name,
				unique:  unique == 1,
				origin:  strings.ToLower(origin),
				partial: partial == 1,
			})
		}
		if err := rows.Err(); err != nil {
			cleanup()
			return nil, fmt.Errorf("iterate sqlite index list rows: %w", err)
		}
		if err := cleanup(); err != nil {
			return nil, fmt.Errorf("close sqlite index list rows: %w", err)
		}

		for _, idx := range pending {
			infoRows, infoCleanup, err := c.query(ctx, fmt.Sprintf(`PRAGMA %s.index_info(%s)`,
				quoteSQLiteIdentifier(schema),
				quoteSQLiteString(idx.name),
			))
			if err != nil {
				return nil, fmt.Errorf("describe sqlite index %s.%s.%s: %w", schema, tableName, idx.name, err)
			}

			item := database.IndexInfo{
				Schema:  schema,
				Table:   tableName,
				Name:    idx.name,
				Unique:  idx.unique,
				Primary: idx.origin == "pk",
				Partial: idx.partial,
				Type:    "btree",
			}
			for infoRows.Next() {
				var (
					seqno int
					cid   int
					name  string
				)
				if err := infoRows.Scan(&seqno, &cid, &name); err != nil {
					infoCleanup()
					return nil, fmt.Errorf("scan sqlite index info row: %w", err)
				}
				_ = seqno
				_ = cid
				if name != "" {
					item.Columns = append(item.Columns, name)
				}
			}
			if err := infoRows.Err(); err != nil {
				infoCleanup()
				return nil, fmt.Errorf("iterate sqlite index info rows: %w", err)
			}
			if err := infoCleanup(); err != nil {
				return nil, fmt.Errorf("close sqlite index info rows: %w", err)
			}

			results = append(results, item)
			if len(results) >= limit {
				return results, nil
			}
		}
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
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}
	if _, err := c.DescribeTable(ctx, schema, table); err != nil {
		return nil, err
	}
	return &database.TableCommentInfo{
		Schema: schema,
		Table:  table,
		Notice: "SQLite does not store table or column comments in system catalogs.",
	}, nil
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

	rows, cleanup, err := c.query(ctx, query)
	if err != nil {
		return nil, fmt.Errorf("describe sqlite table %s.%s: %w", schema, table, err)
	}
	defer cleanup()

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
	rows, cleanup, err := c.query(ctx, normalized)
	if err != nil {
		return nil, fmt.Errorf("execute sqlite query: %w", err)
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
	rows, cleanup, err := c.query(ctx, "EXPLAIN QUERY PLAN "+normalized)
	if err != nil {
		return nil, fmt.Errorf("explain sqlite query: %w", err)
	}
	defer cleanup()

	var lines []string
	for rows.Next() {
		var (
			id      int
			parent  int
			notused int
			detail  string
		)
		if err := rows.Scan(&id, &parent, &notused, &detail); err != nil {
			return nil, fmt.Errorf("scan sqlite explain row: %w", err)
		}
		_ = id
		_ = parent
		_ = notused
		lines = append(lines, detail)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite explain rows: %w", err)
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
