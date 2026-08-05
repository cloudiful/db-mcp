select schemaname::text as schemaname,
       tablename::text as tablename,
       indexname::text as indexname,
       indexdef::text as indexdef
from pg_indexes
where schemaname = $1 and tablename = $2
order by indexname
limit $3
