select schema_name::text as schema_name
from information_schema.schemata
where schema_name not like 'pg_%'
  and schema_name <> 'information_schema'
  and lower(schema_name) like lower($1)
order by schema_name
limit $2
