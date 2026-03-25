package oracle

import (
	"database/sql"
	"fmt"
	"strings"
)

func (c *connection) normalizedSchema(schema string) string {
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	return strings.ToUpper(strings.TrimSpace(schema))
}

func (c *connection) normalizedSchemaTable(schema, table string) (string, string, error) {
	table = strings.ToUpper(strings.TrimSpace(table))
	if table == "" {
		return "", "", fmt.Errorf("table is required")
	}
	return c.normalizedSchema(schema), table, nil
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
