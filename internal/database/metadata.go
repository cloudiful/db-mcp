package database

const metadataNotice = "Metadata was truncated to protect MCP context size."

func MetadataNotice() string {
	return metadataNotice
}

func CompactColumns(columns []ColumnInfo, maxItems, maxTotalChars int) ([]ColumnInfo, bool) {
	if maxItems <= 0 {
		maxItems = len(columns)
	}
	if maxTotalChars <= 0 {
		maxTotalChars = 1 << 30
	}

	total := 0
	result := make([]ColumnInfo, 0, min(len(columns), maxItems))
	for _, column := range columns {
		entryChars := len(column.Name) + len(column.DataType) + 32
		if len(result) >= maxItems || total+entryChars > maxTotalChars {
			return result, true
		}
		total += entryChars
		result = append(result, column)
	}
	return result, false
}

func CompactForeignKeys(keys []ForeignKeyInfo, maxItems, maxTotalChars int) ([]ForeignKeyInfo, bool) {
	if maxItems <= 0 {
		maxItems = len(keys)
	}
	if maxTotalChars <= 0 {
		maxTotalChars = 1 << 30
	}

	total := 0
	result := make([]ForeignKeyInfo, 0, min(len(keys), maxItems))
	for _, key := range keys {
		entryChars := len(key.ConstraintName) + len(key.Schema) + len(key.Table) + len(key.Column) +
			len(key.ReferencedSchema) + len(key.ReferencedTable) + len(key.ReferencedColumn) +
			len(key.UpdateRule) + len(key.DeleteRule) + 32
		if len(result) >= maxItems || total+entryChars > maxTotalChars {
			return result, true
		}
		total += entryChars
		result = append(result, key)
	}
	return result, false
}

func CompactIndexes(indexes []IndexInfo, maxItems, maxTotalChars, maxCellChars int) ([]IndexInfo, bool) {
	if maxItems <= 0 {
		maxItems = len(indexes)
	}
	if maxTotalChars <= 0 {
		maxTotalChars = 1 << 30
	}

	total := 0
	truncated := false
	result := make([]IndexInfo, 0, min(len(indexes), maxItems))
	for _, index := range indexes {
		item := index
		if item.Definition != "" {
			definition, chars, clipped := truncateString(item.Definition, maxCellChars)
			item.Definition = definition
			if clipped {
				truncated = true
			}
			total += chars
		}
		entryChars := len(item.Schema) + len(item.Table) + len(item.Name) + len(item.Type) + 24
		for _, column := range item.Columns {
			entryChars += len(column)
		}
		if len(result) >= maxItems || total+entryChars > maxTotalChars {
			return result, true
		}
		total += entryChars
		result = append(result, item)
	}
	return result, truncated
}

func CompactTableComments(comments *TableCommentInfo, maxItems, maxTotalChars, maxCellChars int) (*TableCommentInfo, bool) {
	if comments == nil {
		return nil, false
	}
	if maxItems <= 0 {
		maxItems = len(comments.Columns)
	}
	if maxTotalChars <= 0 {
		maxTotalChars = 1 << 30
	}

	result := *comments
	result.Columns = nil

	total := 0
	truncated := false
	if result.TableComment != "" {
		comment, chars, clipped := truncateString(result.TableComment, maxCellChars)
		result.TableComment = comment
		total += chars
		if clipped {
			truncated = true
		}
	}

	for _, column := range comments.Columns {
		item := column
		commentChars := 0
		if item.Comment != "" {
			comment, chars, clipped := truncateString(item.Comment, maxCellChars)
			item.Comment = comment
			commentChars = chars
			if clipped {
				truncated = true
			}
		}
		entryChars := len(item.Name) + commentChars + 16
		if len(result.Columns) >= maxItems || total+entryChars > maxTotalChars {
			truncated = true
			break
		}
		total += entryChars
		result.Columns = append(result.Columns, item)
	}

	if truncated {
		result.Notice = AppendNotice(result.Notice, metadataNotice)
	}
	return &result, truncated
}

func AppendNotice(existing, notice string) string {
	if notice == "" {
		return existing
	}
	if existing == "" {
		return notice
	}
	return existing + " " + notice
}
