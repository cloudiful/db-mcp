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
	query := fmt.Sprintf(`
SELECT username
FROM (
  SELECT username
  FROM all_users
  WHERE (:1 IS NULL OR username LIKE '%%' || :1 || '%%')
  ORDER BY username
)
WHERE ROWNUM <= %d`, limit)

	rows, err := c.query(ctx, query, nullableUpper(filter))
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var results []database.SchemaInfo
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, err
		}
		results = append(results, database.SchemaInfo{Name: name})
	}
	return results, rows.Err()
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if schema == "" {
		schema = c.defaultSchema
	}
	query := fmt.Sprintf(`
SELECT owner, object_name, object_type
FROM (
  SELECT owner, object_name, object_type
  FROM all_objects
  WHERE object_type IN ('TABLE', 'VIEW')
    AND generated = 'N'
    AND (:1 IS NULL OR owner = :1)
    AND (:2 IS NULL OR object_name LIKE '%%' || :2 || '%%')
  ORDER BY owner, object_type, object_name
)
WHERE ROWNUM <= %d`, limit)

	rows, err := c.query(ctx, query, nullableUpper(schema), nullableUpper(filter))
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, err
		}
		results = append(results, item)
	}
	return results, rows.Err()
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if table == "" {
		return nil, fmt.Errorf("table is required")
	}
	if schema == "" {
		schema = c.defaultSchema
	}

	rows, err := c.query(ctx, `
SELECT column_id, column_name, data_type, data_length, data_precision, data_scale, nullable
FROM all_tab_columns
WHERE owner = :1 AND table_name = :2
ORDER BY column_id`, strings.ToUpper(schema), strings.ToUpper(table))
	if err != nil {
		return nil, err
	}
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
			return nil, err
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
	return results, rows.Err()
}

func (c *connection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*database.QueryResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	appliedMaxRows := c.limits.NormalizeMaxRows(maxRows)
	startedAt := time.Now()
	rows, err := c.query(ctx, normalized)
	if err != nil {
		return nil, err
	}
	return database.CollectRows(rows, appliedMaxRows, c.limits, startedAt)
}

func (c *connection) Close() error {
	return c.db.Close()
}

func (c *connection) query(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	queryCtx, cancel := c.limits.WithTimeout(ctx)
	defer cancel()
	return c.db.QueryContext(queryCtx, query, args...)
}

func nullableUpper(value string) any {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	return strings.ToUpper(value)
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
