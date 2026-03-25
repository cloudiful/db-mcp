package database

import (
	"database/sql"
	"encoding/hex"
	"fmt"
	"math"
	"strings"
	"time"
	"unicode/utf8"
)

func CollectRows(rows *sql.Rows, maxRows int, limits Limits, startedAt time.Time) (*QueryResult, error) {
	defer rows.Close()

	columnNames, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	columnTypes, err := rows.ColumnTypes()
	if err != nil {
		return nil, err
	}

	visibleColumnCount := len(columnNames)
	truncatedReasons := map[string]struct{}{}
	if visibleColumnCount > limits.MaxColumns {
		visibleColumnCount = limits.MaxColumns
		truncatedReasons["column count exceeded limit"] = struct{}{}
	}

	result := &QueryResult{
		Columns:        make([]QueryColumn, 0, visibleColumnCount),
		Rows:           make([]map[string]any, 0, maxRows),
		AppliedMaxRows: maxRows,
	}
	for i := 0; i < visibleColumnCount; i++ {
		colType := ""
		if i < len(columnTypes) {
			colType = columnTypes[i].DatabaseTypeName()
		}
		result.Columns = append(result.Columns, QueryColumn{Name: columnNames[i], Type: colType})
	}

	totalChars := 0
	for rows.Next() {
		values := make([]any, len(columnNames))
		scanTargets := make([]any, len(columnNames))
		for i := range values {
			scanTargets[i] = &values[i]
		}
		if err := rows.Scan(scanTargets...); err != nil {
			return nil, err
		}

		if len(result.Rows) >= maxRows {
			truncatedReasons["row limit reached"] = struct{}{}
			break
		}

		row := make(map[string]any, visibleColumnCount)
		rowChars := 0
		for i := 0; i < visibleColumnCount; i++ {
			value, chars, truncated := normalizeValue(values[i], limits)
			if truncated {
				truncatedReasons["cell value truncated"] = struct{}{}
			}
			row[columnNames[i]] = value
			rowChars += chars
		}
		if totalChars > 0 && totalChars+rowChars > limits.MaxTotalChars {
			truncatedReasons["total output limit reached"] = struct{}{}
			break
		}
		totalChars += rowChars
		result.Rows = append(result.Rows, row)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	result.RowCount = len(result.Rows)
	result.ExecutionTime = time.Since(startedAt).Round(time.Millisecond).String()
	if len(truncatedReasons) > 0 {
		result.Truncated = true
		result.TruncationReasons = mapKeysSorted(truncatedReasons)
		result.Notice = "Result set was truncated to protect MCP context size."
	}
	return result, nil
}

func normalizeValue(value any, limits Limits) (any, int, bool) {
	switch v := value.(type) {
	case nil:
		return nil, 0, false
	case string:
		return truncateString(v, limits.MaxCellChars)
	case []byte:
		if utf8.Valid(v) {
			return truncateString(string(v), limits.MaxCellChars)
		}
		encoded := hex.EncodeToString(v)
		if len(encoded) > limits.MaxCellChars {
			encoded = encoded[:limits.MaxCellChars] + "...(truncated)"
			return encoded, len(encoded), true
		}
		return encoded, len(encoded), false
	case time.Time:
		formatted := v.UTC().Format(time.RFC3339Nano)
		return formatted, len(formatted), false
	case int64, int32, int16, int8, int:
		text := fmt.Sprint(v)
		return v, len(text), false
	case uint64, uint32, uint16, uint8, uint:
		text := fmt.Sprint(v)
		return v, len(text), false
	case float64:
		if math.IsInf(v, 0) || math.IsNaN(v) {
			text := fmt.Sprint(v)
			return text, len(text), false
		}
		text := fmt.Sprint(v)
		return v, len(text), false
	case float32:
		text := fmt.Sprint(v)
		return v, len(text), false
	case bool:
		text := fmt.Sprint(v)
		return v, len(text), false
	default:
		return truncateString(fmt.Sprint(v), limits.MaxCellChars)
	}
}

func truncateString(value string, maxChars int) (string, int, bool) {
	if utf8.RuneCountInString(value) <= maxChars {
		return value, utf8.RuneCountInString(value), false
	}
	runes := []rune(value)
	trimmed := string(runes[:maxChars]) + "...(truncated)"
	return trimmed, utf8.RuneCountInString(trimmed), true
}

func mapKeysSorted(values map[string]struct{}) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sortStrings(keys)
	return keys
}

func sortStrings(values []string) {
	for i := 0; i < len(values); i++ {
		for j := i + 1; j < len(values); j++ {
			if strings.Compare(values[i], values[j]) > 0 {
				values[i], values[j] = values[j], values[i]
			}
		}
	}
}
