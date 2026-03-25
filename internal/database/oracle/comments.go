package oracle

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"db/internal/database"
)

const (
	tableCommentsQuery = `
SELECT comments
FROM all_tab_comments
WHERE owner = :1 AND table_name = :2`

	columnCommentsQuery = `
SELECT column_name, comments
FROM all_col_comments
WHERE owner = :1 AND table_name = :2
ORDER BY column_name`
)

func (c *connection) GetTableComments(ctx context.Context, schema, table string) (*database.TableCommentInfo, error) {
	var err error
	schema, table, err = c.normalizedSchemaTable(schema, table)
	if err != nil {
		return nil, err
	}

	tableComment, err := c.queryNullableString(ctx, tableCommentsQuery, schema, table)
	if err != nil {
		if errors.Is(err, errOracleRowNotFound) {
			return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
		}
		return nil, fmt.Errorf("get oracle table comment for %s.%s: %w", schema, table, err)
	}

	rows, cleanup, err := c.query(ctx, columnCommentsQuery, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get oracle column comments for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	result := &database.TableCommentInfo{
		Schema:       schema,
		Table:        table,
		TableComment: tableComment,
	}
	for rows.Next() {
		var (
			item       database.ColumnCommentInfo
			commentVal sql.NullString
		)
		if err := rows.Scan(&item.Name, &commentVal); err != nil {
			return nil, fmt.Errorf("scan oracle column comment row: %w", err)
		}
		item.Comment = nullStringValue(commentVal)
		result.Columns = append(result.Columns, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle column comments: %w", err)
	}
	return result, nil
}
