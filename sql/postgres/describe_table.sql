select column_name::text as column_name,
       data_type::text as data_type,
       is_nullable::text as is_nullable,
       ordinal_position,
       character_maximum_length, numeric_precision, numeric_scale
from information_schema.columns
where table_schema = $1 and table_name = $2
order by ordinal_position
