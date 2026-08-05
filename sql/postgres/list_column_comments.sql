select a.attname::text as name, coalesce(d.description, '') as comment
from pg_attribute a
join pg_class c on c.oid = a.attrelid
join pg_namespace n on n.oid = c.relnamespace
left join pg_description d on d.objoid = a.attrelid and d.objsubid = a.attnum
where n.nspname = $1 and c.relname = $2 and a.attnum > 0 and not a.attisdropped
order by a.attnum
