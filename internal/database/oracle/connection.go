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

	pingCtx, cancel := limits.WithTimeout(ctx)
	defer cancel()
	if err := db.PingContext(pingCtx); err != nil {
		_ = db.Close()
		return nil, err
	}

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

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	usernameFilter, usernameArg := optionalLikePredicate("username", 1, filter)
	query := fmt.Sprintf(`
SELECT username
FROM (
  SELECT username
  FROM all_users
  WHERE %s
  ORDER BY username
)
WHERE ROWNUM <= %d`, usernameFilter, limit)

	rows, cleanup, err := c.query(ctx, query, usernameArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle schemas: %w", err)
	}
	defer cleanup()

	var results []database.SchemaInfo
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, fmt.Errorf("scan oracle schema row: %w", err)
		}
		results = append(results, database.SchemaInfo{Name: name})
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle schemas: %w", err)
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if schema == "" {
		schema = c.defaultSchema
	}
	ownerFilter, ownerArg := optionalEqPredicate("owner", 1, schema)
	nameFilter, nameArg := optionalLikePredicate("object_name", 2, filter)
	query := fmt.Sprintf(`
SELECT owner, object_name, object_type
FROM (
  SELECT owner, object_name, object_type
  FROM all_objects
  WHERE object_type IN ('TABLE', 'VIEW')
    AND generated = 'N'
    AND %s
    AND %s
  ORDER BY owner, object_type, object_name
)
WHERE ROWNUM <= %d`, ownerFilter, nameFilter, limit)

	rows, cleanup, err := c.query(ctx, query, ownerArg, nameArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle tables for schema %q: %w", strings.ToUpper(schema), err)
	}
	defer cleanup()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan oracle table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle tables: %w", err)
	}
	return results, nil
}

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if schema == "" {
		schema = c.defaultSchema
	}
	ownerFilter, ownerArg := optionalEqPredicate("owner", 1, schema)
	columnFilter, columnArg := optionalLikePredicate("column_name", 2, filter)
	query := fmt.Sprintf(`
SELECT owner, table_name, column_name, data_type, column_id
FROM (
  SELECT owner, table_name, column_name, data_type, column_id
  FROM all_tab_columns
  WHERE %s
    AND %s
  ORDER BY owner, table_name, column_id
)
WHERE ROWNUM <= %d`, ownerFilter, columnFilter, limit)

	rows, cleanup, err := c.query(ctx, query, ownerArg, columnArg)
	if err != nil {
		return nil, fmt.Errorf("search oracle columns for schema %q: %w", strings.ToUpper(schema), err)
	}
	defer cleanup()

	var results []database.ColumnMatchInfo
	for rows.Next() {
		var item database.ColumnMatchInfo
		if err := rows.Scan(&item.Schema, &item.Table, &item.Column, &item.DataType, &item.Position); err != nil {
			return nil, fmt.Errorf("scan oracle column match row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle column matches: %w", err)
	}
	return results, nil
}

func (c *connection) ListForeignKeys(ctx context.Context, schema, table string, limit int) ([]database.ForeignKeyInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if schema == "" {
		schema = c.defaultSchema
	}
	ownerFilter, ownerArg := optionalEqPredicate("fk.owner", 1, schema)
	tableFilter, tableArg := optionalEqPredicate("fk.table_name", 2, table)
	query := fmt.Sprintf(`
SELECT fk.constraint_name,
       fk.owner,
       fk.table_name,
       fkc.column_name,
       fkc.position,
       pk.owner AS referenced_owner,
       pk.table_name AS referenced_table_name,
       pkc.column_name AS referenced_column_name,
       'NO ACTION' AS update_rule,
       fk.delete_rule
FROM (
  SELECT constraint_name, owner, table_name, r_owner, r_constraint_name, delete_rule
  FROM all_constraints
  WHERE constraint_type = 'R'
    AND %s
    AND %s
) fk
JOIN all_cons_columns fkc
  ON fkc.owner = fk.owner
 AND fkc.constraint_name = fk.constraint_name
JOIN all_constraints pk
  ON pk.owner = fk.r_owner
 AND pk.constraint_name = fk.r_constraint_name
JOIN all_cons_columns pkc
  ON pkc.owner = pk.owner
 AND pkc.constraint_name = pk.constraint_name
 AND pkc.position = fkc.position
ORDER BY fk.owner, fk.table_name, fk.constraint_name, fkc.position`, ownerFilter, tableFilter)

	rows, cleanup, err := c.query(ctx, query, ownerArg, tableArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle foreign keys for schema %q table %q: %w", strings.ToUpper(schema), strings.ToUpper(table), err)
	}
	defer cleanup()

	results := make([]database.ForeignKeyInfo, 0, limit)
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
			return nil, fmt.Errorf("scan oracle foreign key row: %w", err)
		}
		results = append(results, item)
		if len(results) >= limit {
			return results, nil
		}
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle foreign keys: %w", err)
	}
	return results, nil
}

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if schema == "" {
		schema = c.defaultSchema
	}
	ownerFilter, ownerArg := optionalEqPredicate("ai.table_owner", 1, schema)
	tableFilter, tableArg := optionalEqPredicate("ai.table_name", 2, table)
	query := fmt.Sprintf(`
SELECT ai.table_owner,
       ai.table_name,
       ai.index_name,
       ai.index_type,
       ai.uniqueness,
       CASE WHEN pk.index_name IS NULL THEN 0 ELSE 1 END AS is_primary,
       aic.column_position,
       aic.column_name
FROM all_indexes ai
JOIN all_ind_columns aic
  ON aic.index_owner = ai.owner
 AND aic.index_name = ai.index_name
LEFT JOIN (
  SELECT owner, table_name, index_name
  FROM all_constraints
  WHERE constraint_type = 'P'
) pk
  ON pk.owner = ai.table_owner
 AND pk.table_name = ai.table_name
 AND pk.index_name = ai.index_name
WHERE %s
  AND %s
ORDER BY ai.table_owner, ai.table_name, ai.index_name, aic.column_position`, ownerFilter, tableFilter)

	rows, cleanup, err := c.query(ctx, query, ownerArg, tableArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle indexes for schema %q table %q: %w", strings.ToUpper(schema), strings.ToUpper(table), err)
	}
	defer cleanup()

	var (
		results []database.IndexInfo
		current *database.IndexInfo
		lastKey string
	)
	for rows.Next() {
		var (
			item         database.IndexInfo
			uniqueness   string
			isPrimaryNum int
			position     int
			columnName   string
		)
		if err := rows.Scan(
			&item.Schema,
			&item.Table,
			&item.Name,
			&item.Type,
			&uniqueness,
			&isPrimaryNum,
			&position,
			&columnName,
		); err != nil {
			return nil, fmt.Errorf("scan oracle index row: %w", err)
		}
		_ = position
		item.Unique = strings.EqualFold(uniqueness, "UNIQUE")
		item.Primary = isPrimaryNum == 1
		key := item.Schema + "." + item.Table + "." + item.Name
		if key != lastKey {
			if len(results) >= limit {
				return results, nil
			}
			results = append(results, item)
			current = &results[len(results)-1]
			lastKey = key
		}
		if current != nil && columnName != "" {
			current.Columns = append(current.Columns, columnName)
		}
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle indexes: %w", err)
	}
	return results, nil
}

func (c *connection) GetTableComments(ctx context.Context, schema, table string) (*database.TableCommentInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if schema == "" {
		schema = c.defaultSchema
	}
	schema = strings.ToUpper(schema)
	table = strings.ToUpper(strings.TrimSpace(table))

	rows, cleanup, err := c.query(ctx, `
SELECT NVL(comments, '')
FROM all_tab_comments
WHERE owner = :1 AND table_name = :2`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get oracle table comment for %s.%s: %w", schema, table, err)
	}
	result := &database.TableCommentInfo{Schema: schema, Table: table}
	if !rows.Next() {
		cleanup()
		if err := rows.Err(); err != nil {
			return nil, fmt.Errorf("iterate oracle table comments: %w", err)
		}
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	if err := rows.Scan(&result.TableComment); err != nil {
		cleanup()
		return nil, fmt.Errorf("scan oracle table comment: %w", err)
	}
	if err := cleanup(); err != nil {
		return nil, err
	}

	rows, cleanup, err = c.query(ctx, `
SELECT column_name, NVL(comments, '')
FROM all_col_comments
WHERE owner = :1 AND table_name = :2
ORDER BY column_name`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get oracle column comments for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	for rows.Next() {
		var item database.ColumnCommentInfo
		if err := rows.Scan(&item.Name, &item.Comment); err != nil {
			return nil, fmt.Errorf("scan oracle column comment row: %w", err)
		}
		result.Columns = append(result.Columns, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle column comments: %w", err)
	}
	return result, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if table == "" {
		return nil, fmt.Errorf("table is required")
	}
	if schema == "" {
		schema = c.defaultSchema
	}

	rows, cleanup, err := c.query(ctx, `
SELECT column_id, column_name, data_type, data_length, data_precision, data_scale, nullable
FROM all_tab_columns
WHERE owner = :1 AND table_name = :2
ORDER BY column_id`, strings.ToUpper(schema), strings.ToUpper(table))
	if err != nil {
		return nil, fmt.Errorf("describe oracle table %s.%s: %w", strings.ToUpper(schema), strings.ToUpper(table), err)
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
			return nil, fmt.Errorf("scan oracle column row: %w", err)
		}
		item.Nullable = strings.EqualFold(nullable, "Y")
		item.Length = toPtr(length)
		item.Precision = toPtr(precision)
		item.Scale = toPtr(scale)
		results = append(results, item)
	}
	if len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", strings.ToUpper(schema), strings.ToUpper(table))
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle columns: %w", err)
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
	tx, err := c.db.BeginTx(queryCtx, &sql.TxOptions{ReadOnly: true})
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

func nullableUpper(value string) any {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	return strings.ToUpper(value)
}

func nullableLike(value string) any {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	return "%" + strings.ToUpper(value) + "%"
}

func optionalEqPredicate(column string, position int, value string) (string, any) {
	return fmt.Sprintf("%s = NVL(:%d, %s)", column, position, column), nullableUpper(value)
}

func optionalLikePredicate(column string, position int, value string) (string, any) {
	return fmt.Sprintf("%s LIKE NVL(:%d, %s)", column, position, column), nullableLike(value)
}

func toPtr(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	v := value.Int64
	return &v
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
