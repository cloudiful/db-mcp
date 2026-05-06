package mcpserver

import "db/internal/database"

type listConnectionsInput struct {
	Refresh *bool `json:"refresh,omitempty" jsonschema:"optional; defaults to true to probe current connection health before returning"`
}

type listInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; when omitted, tools use the connection's default schema when applicable"`
	Filter     string `json:"filter,omitempty" jsonschema:"optional case-insensitive contains filter"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type describeTableInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table" jsonschema:"table or view name"`
}

type searchColumnsInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Filter     string `json:"filter,omitempty" jsonschema:"optional case-insensitive contains filter for column names"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type listConstraintsInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table,omitempty" jsonschema:"optional table name; when omitted, list foreign keys across the schema"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type listIndexesInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table,omitempty" jsonschema:"optional table name; when omitted, list indexes across the schema"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type getTableCommentsInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table" jsonschema:"table or view name"`
}

type getTableDDLInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table" jsonschema:"table or view name"`
}

type executeQueryInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	SQL        string `json:"sql" jsonschema:"read-only SELECT or WITH query"`
	MaxRows    int    `json:"maxRows,omitempty" jsonschema:"optional row limit; clamped by the server"`
}

type explainQueryInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	SQL        string `json:"sql" jsonschema:"read-only SELECT or WITH query to explain"`
}

type listConnectionsOutput struct {
	DefaultConnection string                    `json:"defaultConnection"`
	Connections       []database.ConnectionInfo `json:"connections"`
}

type listSchemasOutput struct {
	Connection string                `json:"connection"`
	Schemas    []database.SchemaInfo `json:"schemas"`
}

type listTablesOutput struct {
	Connection string               `json:"connection"`
	Tables     []database.TableInfo `json:"tables"`
}

type describeTableOutput struct {
	Connection string                `json:"connection"`
	Schema     string                `json:"schema"`
	Table      string                `json:"table"`
	Columns    []database.ColumnInfo `json:"columns"`
	Notice     string                `json:"notice,omitempty"`
}

type searchColumnsOutput struct {
	Connection string                     `json:"connection"`
	Matches    []database.ColumnMatchInfo `json:"matches"`
}

type listConstraintsOutput struct {
	Connection  string                    `json:"connection"`
	Constraints []database.ConstraintInfo `json:"constraints"`
	Notice      string                    `json:"notice,omitempty"`
}

type listIndexesOutput struct {
	Connection string               `json:"connection"`
	Indexes    []database.IndexInfo `json:"indexes"`
	Notice     string               `json:"notice,omitempty"`
}

type getTableCommentsOutput struct {
	Connection string                     `json:"connection"`
	Comments   *database.TableCommentInfo `json:"comments"`
}

type getTableDDLOutput struct {
	Connection string                 `json:"connection"`
	DDL        *database.TableDDLInfo `json:"ddl"`
}

type executeQueryOutput struct {
	Connection string                `json:"connection"`
	Result     *database.QueryResult `json:"result"`
}

type explainQueryOutput struct {
	Connection string                  `json:"connection"`
	Explain    *database.ExplainResult `json:"explain"`
}

type tableResourceOutput struct {
	Connection  string                      `json:"connection"`
	Schema      string                      `json:"schema"`
	Table       string                      `json:"table"`
	Columns     []database.ColumnInfo       `json:"columns"`
	Constraints []database.ConstraintInfo   `json:"constraints"`
	Indexes     []database.IndexInfo        `json:"indexes"`
	Comments    *database.TableCommentInfo  `json:"comments"`
	Notice      string                      `json:"notice,omitempty"`
}
