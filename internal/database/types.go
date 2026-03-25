package database

import (
	"context"
	"fmt"
	"sort"
	"time"

	"db/internal/config"
)

type Connection interface {
	Info() ConnectionInfo
	ListSchemas(ctx context.Context, filter string, limit int) ([]SchemaInfo, error)
	ListTables(ctx context.Context, schema, filter string, limit int) ([]TableInfo, error)
	SearchColumns(ctx context.Context, schema, filter string, limit int) ([]ColumnMatchInfo, error)
	ListForeignKeys(ctx context.Context, schema, table string, limit int) ([]ForeignKeyInfo, error)
	ListIndexes(ctx context.Context, schema, table string, limit int) ([]IndexInfo, error)
	GetTableComments(ctx context.Context, schema, table string) (*TableCommentInfo, error)
	DescribeTable(ctx context.Context, schema, table string) ([]ColumnInfo, error)
	ExecuteQuery(ctx context.Context, query string, maxRows int) (*QueryResult, error)
	ExplainQuery(ctx context.Context, query string) (*ExplainResult, error)
	Close() error
}

type OpenFunc func(ctx context.Context, cfg config.ConnectionConfig, limits Limits) (Connection, error)

type ConnectionInfo struct {
	Name          string `json:"name"`
	Driver        string `json:"driver"`
	DefaultSchema string `json:"defaultSchema"`
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
	DefaultMaxRows int
	HardMaxRows    int
	MaxColumns     int
	MaxCellChars   int
	MaxTotalChars  int
	QueryTimeout   time.Duration
}

func NewLimits(cfg *config.Config) Limits {
	return Limits{
		DefaultMaxRows: cfg.Limits.DefaultMaxRows,
		HardMaxRows:    cfg.Limits.HardMaxRows,
		MaxColumns:     cfg.Limits.MaxColumns,
		MaxCellChars:   cfg.Limits.MaxCellChars,
		MaxTotalChars:  cfg.Limits.MaxTotalChars,
		QueryTimeout:   cfg.QueryTimeout(),
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

type Manager struct {
	connections map[string]Connection
	defaultName string
	limits      Limits
}

var registry = map[string]OpenFunc{}

func Register(driver string, open OpenFunc) {
	registry[driver] = open
}

func NewManager(ctx context.Context, cfg *config.Config) (*Manager, error) {
	limits := NewLimits(cfg)
	connections := make(map[string]Connection, len(cfg.Connections))
	for _, connCfg := range cfg.Connections {
		open, ok := registry[connCfg.Driver]
		if !ok {
			closeConnections(connections)
			return nil, fmt.Errorf("driver %q is not registered", connCfg.Driver)
		}
		conn, err := open(ctx, connCfg, limits)
		if err != nil {
			closeConnections(connections)
			return nil, fmt.Errorf("open %s: %w", connCfg.Name, err)
		}
		connections[connCfg.Name] = conn
	}
	return &Manager{
		connections: connections,
		defaultName: cfg.Connections[0].Name,
		limits:      limits,
	}, nil
}

func closeConnections(connections map[string]Connection) {
	for _, conn := range connections {
		_ = conn.Close()
	}
}

func (m *Manager) Limits() Limits {
	return m.limits
}

func (m *Manager) Connection(name string) (Connection, error) {
	if name == "" {
		name = m.defaultName
	}
	conn, ok := m.connections[name]
	if !ok {
		return nil, fmt.Errorf("connection %q not found", name)
	}
	return conn, nil
}

func (m *Manager) ConnectionInfos() []ConnectionInfo {
	infos := make([]ConnectionInfo, 0, len(m.connections))
	for _, conn := range m.connections {
		infos = append(infos, conn.Info())
	}
	sort.Slice(infos, func(i, j int) bool {
		return infos[i].Name < infos[j].Name
	})
	return infos
}

func (m *Manager) DefaultConnectionName() string {
	return m.defaultName
}

func (m *Manager) Close() error {
	var firstErr error
	for _, conn := range m.connections {
		if err := conn.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}
