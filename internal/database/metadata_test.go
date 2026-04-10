package database

import (
	"strings"
	"testing"
)

func TestCompactColumnsRespectsBudget(t *testing.T) {
	t.Parallel()

	columns := []ColumnInfo{
		{Name: "alpha", DataType: "text"},
		{Name: "beta", DataType: "text"},
		{Name: "gamma", DataType: "text"},
	}

	result, truncated := CompactColumns(columns, 10, 60)
	if !truncated {
		t.Fatalf("expected columns to be truncated")
	}
	if len(result) != 1 {
		t.Fatalf("expected one column to fit budget, got %d", len(result))
	}
}

func TestCompactIndexesTruncatesDefinitions(t *testing.T) {
	t.Parallel()

	indexes := []IndexInfo{{
		Schema:     "main",
		Table:      "events",
		Name:       "idx_events_payload",
		Definition: strings.Repeat("x", 40),
	}}

	result, truncated := CompactIndexes(indexes, 10, 200, 12)
	if !truncated {
		t.Fatalf("expected index definition truncation")
	}
	if len(result) != 1 {
		t.Fatalf("expected one index result")
	}
	if !strings.Contains(result[0].Definition, "...(truncated)") {
		t.Fatalf("expected truncated definition, got %q", result[0].Definition)
	}
}

func TestCompactTableCommentsAppendsNotice(t *testing.T) {
	t.Parallel()

	comments := &TableCommentInfo{
		Schema:       "main",
		Table:        "events",
		TableComment: strings.Repeat("t", 40),
		Columns: []ColumnCommentInfo{
			{Name: "payload", Comment: strings.Repeat("c", 40)},
			{Name: "source", Comment: strings.Repeat("d", 40)},
		},
	}

	result, truncated := CompactTableComments(comments, 10, 80, 16)
	if !truncated {
		t.Fatalf("expected comment compaction to truncate")
	}
	if result == nil {
		t.Fatalf("expected comment result")
	}
	if !strings.Contains(result.Notice, MetadataNotice()) {
		t.Fatalf("expected truncation notice, got %q", result.Notice)
	}
	if !strings.Contains(result.TableComment, "...(truncated)") {
		t.Fatalf("expected table comment truncation, got %q", result.TableComment)
	}
}
