select obj_description((quote_ident($1) || '.' || quote_ident($2))::regclass::oid, 'pg_class') as table_comment
