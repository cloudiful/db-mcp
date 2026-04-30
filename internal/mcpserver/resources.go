package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/database"
)

func (s *service) readConnectionsResource(_ context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
	if req.Params.URI != "db://connections" {
		return nil, mcp.ResourceNotFoundError(req.Params.URI)
	}
	body, err := json.Marshal(&listConnectionsOutput{
		DefaultConnection: s.manager.DefaultConnectionName(),
		Connections:       s.manager.ConnectionInfos(),
	})
	if err != nil {
		return nil, err
	}
	return &mcp.ReadResourceResult{
		Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "application/json",
			Text:     string(body),
		}},
	}, nil
}

func (s *service) readHealthResource(ctx context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
	if req.Params.URI != "db://health" {
		return nil, mcp.ResourceNotFoundError(req.Params.URI)
	}
	snapshot, err := s.manager.Health(ctx, "", true)
	if err != nil {
		return nil, err
	}
	body, err := json.Marshal(&getHealthOutput{
		Status:            snapshot.Status,
		DefaultConnection: snapshot.DefaultConnection,
		Connections:       snapshot.Connections,
	})
	if err != nil {
		return nil, err
	}
	return &mcp.ReadResourceResult{
		Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "application/json",
			Text:     string(body),
		}},
	}, nil
}

func (s *service) readTableResource(ctx context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
	connection, schema, table, err := parseTableResourceURI(req.Params.URI)
	if err != nil {
		return nil, mcp.ResourceNotFoundError(req.Params.URI)
	}
	body, err := s.buildTableResource(ctx, connection, schema, table)
	if err != nil {
		return nil, err
	}
	return &mcp.ReadResourceResult{
		Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "application/json",
			Text:     body,
		}},
	}, nil
}

func (s *service) buildTableResource(ctx context.Context, connectionName, schema, table string) (string, error) {
	conn, err := s.manager.Connection(connectionName)
	if err != nil {
		return "", mcp.ResourceNotFoundError("db://table/" + connectionName + "/" + schema + "/" + table)
	}
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	limits := s.manager.Limits()
	columns, err := conn.DescribeTable(ctx, schema, table)
	if err != nil {
		return "", err
	}
	sectionBudget := max(256, limits.MaxTotalChars/4)
	columns, columnsTruncated := database.CompactColumns(columns, limits.HardMaxRows, sectionBudget)
	foreignKeys, err := conn.ListForeignKeys(ctx, schema, table, limits.HardMaxRows)
	if err != nil {
		return "", err
	}
	foreignKeys, foreignKeysTruncated := database.CompactForeignKeys(foreignKeys, limits.HardMaxRows, sectionBudget)
	indexes, err := conn.ListIndexes(ctx, schema, table, limits.HardMaxRows)
	if err != nil {
		return "", err
	}
	indexes, indexesTruncated := database.CompactIndexes(indexes, limits.HardMaxRows, sectionBudget, limits.MaxCellChars)
	comments, err := conn.GetTableComments(ctx, schema, table)
	if err != nil {
		return "", err
	}
	comments, commentsTruncated := database.CompactTableComments(comments, limits.HardMaxRows, sectionBudget, limits.MaxCellChars)
	notice := ""
	if columnsTruncated || foreignKeysTruncated || indexesTruncated || commentsTruncated {
		notice = database.MetadataNotice()
	}
	body, err := json.Marshal(&tableResourceOutput{
		Connection:  conn.Info().Name,
		Schema:      schema,
		Table:       table,
		Columns:     columns,
		ForeignKeys: foreignKeys,
		Indexes:     indexes,
		Comments:    comments,
		Notice:      notice,
	})
	if err != nil {
		return "", err
	}
	return string(body), nil
}

func parseTableResourceURI(raw string) (connection string, schema string, table string, err error) {
	parsed, err := url.Parse(raw)
	if err != nil {
		return "", "", "", err
	}
	if parsed.Scheme != "db" || parsed.Host != "table" {
		return "", "", "", fmt.Errorf("unsupported resource uri %q", raw)
	}
	parts := strings.Split(strings.TrimPrefix(parsed.Path, "/"), "/")
	if len(parts) != 3 {
		return "", "", "", fmt.Errorf("invalid table resource uri %q", raw)
	}
	return parts[0], parts[1], parts[2], nil
}
