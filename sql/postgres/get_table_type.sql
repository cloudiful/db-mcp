select table_type
from information_schema.tables
where table_schema = $1 and table_name = $2
