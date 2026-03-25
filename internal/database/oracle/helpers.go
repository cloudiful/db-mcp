package oracle

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"

	"db/internal/database"
)

var errOracleRowNotFound = errors.New("oracle row not found")

func (c *connection) normalizedSchema(schema string) string {
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	return strings.ToUpper(strings.TrimSpace(schema))
}

func (c *connection) normalizedSchemaTable(schema, table string) (string, string, error) {
	table = normalizedOptionalName(table)
	if table == "" {
		return "", "", fmt.Errorf("table is required")
	}
	return c.normalizedSchema(schema), table, nil
}

func normalizedOptionalName(value string) string {
	return strings.ToUpper(strings.TrimSpace(value))
}

func (c *connection) queryNullableString(ctx context.Context, query string, args ...any) (string, error) {
	rows, cleanup, err := c.query(ctx, query, args...)
	if err != nil {
		return "", err
	}
	if !rows.Next() {
		cleanup()
		if err := rows.Err(); err != nil {
			return "", err
		}
		return "", errOracleRowNotFound
	}
	var value sql.NullString
	if err := rows.Scan(&value); err != nil {
		cleanup()
		return "", err
	}
	if err := cleanup(); err != nil {
		return "", err
	}
	return nullStringValue(value), nil
}

func appendIndexColumn(item *database.IndexInfo, value sql.NullString) {
	if item == nil {
		return
	}
	column := nullStringValue(value)
	if column != "" {
		item.Columns = append(item.Columns, column)
	}
}

func nullStringValue(value sql.NullString) string {
	if !value.Valid {
		return ""
	}
	return value.String
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
