select table_schema::text as table_schema,
       table_name::text as table_name,
       column_name::text as column_name,
       data_type::text as data_type,
       ordinal_position
from information_schema.columns
where table_schema = $1
  and lower(column_name) like lower($2)
order by table_name, ordinal_position
limit $3
