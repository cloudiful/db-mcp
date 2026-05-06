package database

import (
	"context"
	"time"

	"db/internal/config"
)

type Connection interface {
	Info() ConnectionInfo
	Ping(ctx context.Context) error
	ListSchemas(ctx context.Context, filter string, limit int) ([]SchemaInfo, error)
	ListTables(ctx context.Context, schema, filter string, limit int) ([]TableInfo, error)
	SearchColumns(ctx context.Context, schema, filter string, limit int) ([]ColumnMatchInfo, error)
	ListConstraints(ctx context.Context, schema, table string, limit int) ([]ConstraintInfo, error)
	ListIndexes(ctx context.Context, schema, table string, limit int) ([]IndexInfo, error)
	GetTableComments(ctx context.Context, schema, table string) (*TableCommentInfo, error)
	DescribeTable(ctx context.Context, schema, table string) ([]ColumnInfo, error)
	GetTableDDL(ctx context.Context, schema, table string) (*TableDDLInfo, error)
	ExecuteQuery(ctx context.Context, query string, maxRows int) (*QueryResult, error)
	ExplainQuery(ctx context.Context, query string) (*ExplainResult, error)
	Close() error
}

type OpenFunc func(ctx context.Context, cfg config.ConnectionConfig, limits Limits) (Connection, error)

type ConnectionInfo struct {
	Name             string     `json:"name"`
	Driver           string     `json:"driver"`
	DefaultSchema    string     `json:"defaultSchema"`
	Status           string     `json:"status,omitempty"`
	Available        bool       `json:"available"`
	LastCheckedAt    *time.Time `json:"lastCheckedAt,omitempty"`
	LastSuccessfulAt *time.Time `json:"lastSuccessfulAt,omitempty"`
	LastError        string     `json:"lastError,omitempty"`
}

type ConnectionHealth struct {
	Name             string     `json:"name"`
	Driver           string     `json:"driver"`
	DefaultSchema    string     `json:"defaultSchema"`
	Status           string     `json:"status"`
	Available        bool       `json:"available"`
	LastCheckedAt    *time.Time `json:"lastCheckedAt,omitempty"`
	LastSuccessfulAt *time.Time `json:"lastSuccessfulAt,omitempty"`
	LastError        string     `json:"lastError,omitempty"`
}

type HealthSnapshot struct {
	Status            string             `json:"status"`
	DefaultConnection string             `json:"defaultConnection"`
	Connections       []ConnectionHealth `json:"connections"`
}

type SchemaInfo struct {
	Name string `json:"name"`
}

type TableInfo struct {
	Schema string `json:"schema"`
	Name   string `json:"name"`
	Type   string `json:"type"`
}

type ColumnInfo struct {
	Name      string `json:"name"`
	DataType  string `json:"dataType"`
	Nullable  bool   `json:"nullable"`
	Position  int    `json:"position"`
	Length    *int64 `json:"length,omitempty"`
	Precision *int64 `json:"precision,omitempty"`
	Scale     *int64 `json:"scale,omitempty"`
}

type ColumnMatchInfo struct {
	Schema   string `json:"schema"`
	Table    string `json:"table"`
	Column   string `json:"column"`
	DataType string `json:"dataType"`
	Position int    `json:"position"`
}

type ForeignKeyInfo struct {
	ConstraintName   string `json:"constraintName,omitempty"`
	Schema           string `json:"schema"`
	Table            string `json:"table"`
	Column           string `json:"column"`
	Position         int    `json:"position"`
	ReferencedSchema string `json:"referencedSchema,omitempty"`
	ReferencedTable  string `json:"referencedTable"`
	ReferencedColumn string `json:"referencedColumn,omitempty"`
	UpdateRule       string `json:"updateRule,omitempty"`
	DeleteRule       string `json:"deleteRule,omitempty"`
}

type ConstraintInfo struct {
	Name              string   `json:"name,omitempty"`
	Schema            string   `json:"schema"`
	Table             string   `json:"table"`
	Type              string   `json:"type"`
	Columns           []string `json:"columns,omitempty"`
	ReferencedSchema  string   `json:"referencedSchema,omitempty"`
	ReferencedTable   string   `json:"referencedTable,omitempty"`
	ReferencedColumns []string `json:"referencedColumns,omitempty"`
	CheckExpression   string   `json:"checkExpression,omitempty"`
	Definition        string   `json:"definition,omitempty"`
}

type IndexInfo struct {
	Schema     string   `json:"schema"`
	Table      string   `json:"table"`
	Name       string   `json:"name"`
	Type       string   `json:"type,omitempty"`
	Unique     bool     `json:"unique"`
	Primary    bool     `json:"primary"`
	Partial    bool     `json:"partial,omitempty"`
	Columns    []string `json:"columns,omitempty"`
	Definition string   `json:"definition,omitempty"`
}

type ColumnCommentInfo struct {
	Name    string `json:"name"`
	Comment string `json:"comment,omitempty"`
}

type TableCommentInfo struct {
	Schema       string              `json:"schema"`
	Table        string              `json:"table"`
	TableComment string              `json:"tableComment,omitempty"`
	Columns      []ColumnCommentInfo `json:"columns,omitempty"`
	Notice       string              `json:"notice,omitempty"`
}

type TableDDLInfo struct {
	Schema     string `json:"schema"`
	Table      string `json:"table"`
	ObjectType string `json:"objectType"`
	DDL        string `json:"ddl"`
	Notice     string `json:"notice,omitempty"`
}

type ExplainResult struct {
	Format    string   `json:"format"`
	PlanLines []string `json:"planLines"`
	Notice    string   `json:"notice,omitempty"`
}

type QueryColumn struct {
	Name string `json:"name"`
	Type string `json:"type"`
}

type QueryResult struct {
	Columns           []QueryColumn    `json:"columns"`
	Rows              []map[string]any `json:"rows"`
	RowCount          int              `json:"rowCount"`
	AppliedMaxRows    int              `json:"appliedMaxRows"`
	Truncated         bool             `json:"truncated"`
	TruncationReasons []string         `json:"truncationReasons,omitempty"`
	Notice            string           `json:"notice,omitempty"`
	ExecutionTime     string           `json:"executionTime"`
}

type Limits struct {
	DefaultMaxRows       int
	HardMaxRows          int
	MaxColumns           int
	MaxCellChars         int
	MaxTotalChars        int
	ExplainMaxLines      int
	ExplainMaxTotalChars int
	QueryTimeout         time.Duration
}

func NewLimits(cfg *config.Config) Limits {
	return Limits{
		DefaultMaxRows:       cfg.Limits.DefaultMaxRows,
		HardMaxRows:          cfg.Limits.HardMaxRows,
		MaxColumns:           cfg.Limits.MaxColumns,
		MaxCellChars:         cfg.Limits.MaxCellChars,
		MaxTotalChars:        cfg.Limits.MaxTotalChars,
		ExplainMaxLines:      cfg.Limits.ExplainMaxLines,
		ExplainMaxTotalChars: cfg.Limits.ExplainMaxTotalChars,
		QueryTimeout:         cfg.QueryTimeout(),
	}
}

func (l Limits) NormalizeMaxRows(requested int) int {
	if requested <= 0 {
		return l.DefaultMaxRows
	}
	if requested > l.HardMaxRows {
		return l.HardMaxRows
	}
	return requested
}

func (l Limits) NormalizeListLimit(requested int) int {
	limit := requested
	if limit <= 0 {
		limit = l.DefaultMaxRows
	}
	if limit > l.HardMaxRows {
		limit = l.HardMaxRows
	}
	return limit
}

func (l Limits) WithTimeout(ctx context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(ctx, l.QueryTimeout)
}
