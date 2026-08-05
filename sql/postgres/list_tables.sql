select table_schema::text as table_schema,
       table_name::text as table_name,
       table_type::text as table_type
from information_schema.tables
where table_schema = $1
  and lower(table_name) like lower($2)
  and table_type in ('BASE TABLE', 'VIEW')
order by table_name
limit $3
