package mcpserver

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	"db/internal/config"
	"db/internal/database"
	_ "db/internal/database/sqlite"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type stubUnavailableConnection struct {
	name    string
	driver  string
	schema  string
	pingErr error
}

func (s *stubUnavailableConnection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{Name: s.name, Driver: s.driver, DefaultSchema: s.schema}
}

func (s *stubUnavailableConnection) Ping(context.Context) error {
	return s.pingErr
}

func (s *stubUnavailableConnection) ListSchemas(context.Context, string, int) ([]database.SchemaInfo, error) {
	return []database.SchemaInfo{{Name: s.schema}}, nil
}

func (s *stubUnavailableConnection) ListTables(context.Context, string, string, int) ([]database.TableInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) SearchColumns(context.Context, string, string, int) ([]database.ColumnMatchInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) ListConstraints(context.Context, string, string, int) ([]database.ConstraintInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) ListIndexes(context.Context, string, string, int) ([]database.IndexInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) GetTableComments(context.Context, string, string) (*database.TableCommentInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) DescribeTable(context.Context, string, string) ([]database.ColumnInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) GetTableDDL(context.Context, string, string) (*database.TableDDLInfo, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) ExecuteQuery(context.Context, string, int) (*database.QueryResult, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) ExplainQuery(context.Context, string) (*database.ExplainResult, error) {
	return nil, nil
}

func (s *stubUnavailableConnection) Close() error {
	return nil
}

func newSQLiteTestManager(t *testing.T) (*config.Config, *database.Manager) {
	tempDir := t.TempDir()
	dbPath := filepath.Join(tempDir, "test.db")

	writableDB, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatalf("open writable sqlite db: %v", err)
	}
	defer writableDB.Close()

	if _, err := writableDB.Exec(`CREATE TABLE user_profile (id INTEGER PRIMARY KEY, display_name TEXT)`); err != nil {
		t.Fatalf("create table: %v", err)
	}
	if _, err := writableDB.Exec(`CREATE TABLE purchase_order (
		id INTEGER PRIMARY KEY,
		user_profile_id INTEGER NOT NULL,
		amount DECIMAL(10,2),
		FOREIGN KEY (user_profile_id) REFERENCES user_profile(id) ON DELETE CASCADE
	)`); err != nil {
		t.Fatalf("create foreign-key table: %v", err)
	}
	if _, err := writableDB.Exec(`CREATE INDEX idx_purchase_order_user_profile ON purchase_order(user_profile_id)`); err != nil {
		t.Fatalf("create index: %v", err)
	}

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{{
			Name:   "sqlite-test",
			Driver: "sqlite",
			SQLite: &config.SQLiteConfig{
				DSN: "file:" + dbPath + "?mode=ro",
			},
		}},
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}

	manager, err := database.NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	return cfg, manager
}

func TestDescribeTablePreservesRequestedCaseForNonOracleDrivers(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.describeTable(context.Background(), nil, describeTableInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Table:      "user_profile",
	})
	if err != nil {
		t.Fatalf("describe table: %v", err)
	}
	if output.Schema != "main" {
		t.Fatalf("expected schema to preserve case, got %q", output.Schema)
	}
	if output.Table != "user_profile" {
		t.Fatalf("expected table to preserve case, got %q", output.Table)
	}
}

func TestSearchColumnsFindsMatchesInSQLite(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.searchColumns(context.Background(), nil, searchColumnsInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Filter:     "display",
	})
	if err != nil {
		t.Fatalf("search columns: %v", err)
	}
	if len(output.Matches) != 1 {
		t.Fatalf("expected one match, got %#v", output.Matches)
	}
	if output.Matches[0].Table != "user_profile" || output.Matches[0].Column != "display_name" {
		t.Fatalf("unexpected match %#v", output.Matches[0])
	}
}

func TestListConstraintsFindsRelationshipsInSQLite(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.listConstraints(context.Background(), nil, listConstraintsInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Table:      "purchase_order",
	})
	if err != nil {
		t.Fatalf("list constraints: %v", err)
	}
	if len(output.Constraints) == 0 {
		t.Fatalf("expected at least one constraint row, got %#v", output.Constraints)
	}
	foundFK := false
	for _, got := range output.Constraints {
		if got.Type != "FOREIGN KEY" {
			continue
		}
		foundFK = true
		if got.Table != "purchase_order" || len(got.Columns) != 1 || got.Columns[0] != "user_profile_id" || got.ReferencedTable != "user_profile" || len(got.ReferencedColumns) != 1 || got.ReferencedColumns[0] != "id" {
			t.Fatalf("unexpected foreign key %#v", got)
		}
	}
	if !foundFK {
		t.Fatalf("expected foreign key in constraints %#v", output.Constraints)
	}
}

func TestListIndexesFindsIndexesInSQLite(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.listIndexes(context.Background(), nil, listIndexesInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Table:      "purchase_order",
	})
	if err != nil {
		t.Fatalf("list indexes: %v", err)
	}
	if len(output.Indexes) == 0 {
		t.Fatalf("expected at least one index")
	}
	found := false
	for _, index := range output.Indexes {
		if index.Name == "idx_purchase_order_user_profile" {
			found = true
			if len(index.Columns) != 1 || index.Columns[0] != "user_profile_id" {
				t.Fatalf("unexpected index columns %#v", index.Columns)
			}
		}
	}
	if !found {
		t.Fatalf("expected explicit secondary index, got %#v", output.Indexes)
	}
}

func TestGetTableCommentsReturnsSQLiteNotice(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.getTableComments(context.Background(), nil, getTableCommentsInput{
		Connection: "sqlite-test",
		Schema:     "main",
		Table:      "purchase_order",
	})
	if err != nil {
		t.Fatalf("get table comments: %v", err)
	}
	if output.Comments == nil || !strings.Contains(output.Comments.Notice, "SQLite") {
		t.Fatalf("expected sqlite notice, got %#v", output.Comments)
	}
}

func TestExplainQueryReturnsPlanForSQLite(t *testing.T) {
	t.Parallel()

	_, manager := newSQLiteTestManager(t)
	defer manager.Close()

	svc := &service{manager: manager}
	_, output, err := svc.explainQuery(context.Background(), nil, explainQueryInput{
		Connection: "sqlite-test",
		SQL:        "SELECT user_profile_id FROM purchase_order WHERE user_profile_id = 1",
	})
	if err != nil {
		t.Fatalf("explain query: %v", err)
	}
	if output.Explain == nil || len(output.Explain.PlanLines) == 0 {
		t.Fatalf("expected explain lines, got %#v", output.Explain)
	}
}

func TestServerRegistersInstructionsAndPrompts(t *testing.T) {
	t.Parallel()

	cfg, manager := newSQLiteTestManager(t)
	defer manager.Close()

	server := New(cfg, manager)
	client := mcp.NewClient(&mcp.Implementation{Name: "test-client", Version: "v0.0.1"}, nil)
	t1, t2 := mcp.NewInMemoryTransports()
	if _, err := server.Connect(context.Background(), t1, nil); err != nil {
		t.Fatalf("connect server: %v", err)
	}
	session, err := client.Connect(context.Background(), t2, nil)
	if err != nil {
		t.Fatalf("connect client: %v", err)
	}
	defer session.Close()

	initResult := session.InitializeResult()
	if initResult == nil {
		t.Fatalf("expected initialize result")
	}
	if !strings.Contains(initResult.Instructions, "search_columns") || !strings.Contains(initResult.Instructions, "list_constraints") {
		t.Fatalf("expected instructions to mention search_columns, got %q", initResult.Instructions)
	}
	if strings.Contains(initResult.Instructions, "get_health") || strings.Contains(initResult.Instructions, "db://health") {
		t.Fatalf("expected instructions to remove legacy health checks, got %q", initResult.Instructions)
	}
	if !strings.Contains(initResult.Instructions, "db://table/{connection}/{schema}/{table}") {
		t.Fatalf("expected instructions to mention table resource, got %q", initResult.Instructions)
	}

	prompts, err := session.ListPrompts(context.Background(), nil)
	if err != nil {
		t.Fatalf("list prompts: %v", err)
	}
	if len(prompts.Prompts) != 2 {
		t.Fatalf("expected 2 prompts, got %d", len(prompts.Prompts))
	}

	result, err := session.GetPrompt(context.Background(), &mcp.GetPromptParams{
		Name:      "inspect_table",
		Arguments: map[string]string{"table": "user_profile", "schema": "main"},
	})
	if err != nil {
		t.Fatalf("get prompt: %v", err)
	}
	if len(result.Messages) != 1 {
		t.Fatalf("expected one prompt message, got %d", len(result.Messages))
	}
	content, ok := result.Messages[0].Content.(*mcp.TextContent)
	if !ok {
		t.Fatalf("expected text content, got %T", result.Messages[0].Content)
	}
	if !strings.Contains(content.Text, "describe_table") || !strings.Contains(content.Text, "list_constraints") || !strings.Contains(content.Text, "execute_query") {
		t.Fatalf("unexpected prompt text %q", content.Text)
	}

	resources, err := session.ListResources(context.Background(), nil)
	if err != nil {
		t.Fatalf("list resources: %v", err)
	}
	if len(resources.Resources) != 1 {
		t.Fatalf("unexpected resources %#v", resources.Resources)
	}
	if resources.Resources[0].URI != "db://connections" {
		t.Fatalf("unexpected resources %#v", resources.Resources)
	}

	templates, err := session.ListResourceTemplates(context.Background(), nil)
	if err != nil {
		t.Fatalf("list resource templates: %v", err)
	}
	if len(templates.ResourceTemplates) != 1 || templates.ResourceTemplates[0].URITemplate != "db://table/{connection}/{schema}/{table}" {
		t.Fatalf("unexpected resource templates %#v", templates.ResourceTemplates)
	}

	tableResource, err := session.ReadResource(context.Background(), &mcp.ReadResourceParams{
		URI: "db://table/sqlite-test/main/purchase_order",
	})
	if err != nil {
		t.Fatalf("read table resource: %v", err)
	}
	if len(tableResource.Contents) != 1 || !strings.Contains(tableResource.Contents[0].Text, "idx_purchase_order_user_profile") {
		t.Fatalf("unexpected table resource %#v", tableResource.Contents)
	}
	if strings.Contains(tableResource.Contents[0].Text, "\n") {
		t.Fatalf("expected compact json resource, got %q", tableResource.Contents[0].Text)
	}

	if !strings.Contains(tableResource.Contents[0].Text, "\"constraints\"") {
		t.Fatalf("expected constraints in table resource, got %#v", tableResource.Contents)
	}
}

func TestListConnectionsReportsMixedAvailability(t *testing.T) {
	t.Parallel()

	healthyDriver := "mcpserver-health-healthy"
	downDriver := "mcpserver-health-down"
	database.Register(healthyDriver, func(context.Context, config.ConnectionConfig, database.Limits) (database.Connection, error) {
		return &stubUnavailableConnection{name: "healthy", driver: "stub", schema: "main"}, nil
	})
	database.Register(downDriver, func(context.Context, config.ConnectionConfig, database.Limits) (database.Connection, error) {
		return &stubUnavailableConnection{name: "down", driver: "stub", schema: "main", pingErr: errors.New("dial timeout")}, nil
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "healthy", Driver: healthyDriver},
			{Name: "down", Driver: downDriver},
		},
	}
	manager, err := database.NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	svc := &service{manager: manager}
	refresh := false
	_, output, err := svc.listConnections(context.Background(), nil, listConnectionsInput{Refresh: &refresh})
	if err != nil {
		t.Fatalf("list connections: %v", err)
	}
	if len(output.Connections) != 2 {
		t.Fatalf("expected two connections, got %#v", output.Connections)
	}
	statusByName := map[string]string{}
	for _, conn := range output.Connections {
		statusByName[conn.Name] = conn.Status
	}
	if statusByName["healthy"] != database.StatusUp || statusByName["down"] != database.StatusDown {
		t.Fatalf("unexpected connection statuses %#v", output.Connections)
	}
}

func TestDownConnectionReturnsUnavailableToolError(t *testing.T) {
	t.Parallel()

	driver := "mcpserver-down-only"
	database.Register(driver, func(context.Context, config.ConnectionConfig, database.Limits) (database.Connection, error) {
		return &stubUnavailableConnection{name: "down", driver: "stub", schema: "main", pingErr: errors.New("db is down")}, nil
	})

	cfg := &config.Config{
		Server: config.ServerConfig{},
		Limits: config.LimitsConfig{},
		Connections: []config.ConnectionConfig{
			{Name: "down", Driver: driver},
		},
	}
	manager, err := database.NewManager(context.Background(), cfg)
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}
	defer manager.Close()

	svc := &service{manager: manager}
	result, output, err := svc.listSchemas(context.Background(), nil, listInput{Connection: "down"})
	if err != nil {
		t.Fatalf("list schemas returned outer error: %v", err)
	}
	if output != nil {
		t.Fatalf("expected no structured output on tool error, got %#v", output)
	}
	if result == nil || !result.IsError {
		t.Fatalf("expected tool error result, got %#v", result)
	}
	content, ok := result.Content[0].(*mcp.TextContent)
	if !ok || !strings.Contains(content.Text, "unavailable") {
		t.Fatalf("expected unavailable error text, got %#v", result.Content)
	}
}
