select
    c.conname::text as name,
    n.nspname::text as schema,
    rel.relname::text as table_name,
    case c.contype
        when 'p' then 'PRIMARY KEY'
        when 'u' then 'UNIQUE'
        when 'f' then 'FOREIGN KEY'
        when 'c' then 'CHECK'
        else c.contype::text
    end as constraint_type,
    pg_get_constraintdef(c.oid, true)::text as definition,
    case when c.confrelid <> 0 then nconf.nspname::text end as referenced_schema,
    case when c.confrelid <> 0 then relconf.relname::text end as referenced_table
from pg_constraint c
join pg_class rel on rel.oid = c.conrelid
join pg_namespace n on n.oid = rel.relnamespace
left join pg_class relconf on relconf.oid = c.confrelid
left join pg_namespace nconf on nconf.oid = relconf.relnamespace
where n.nspname = $1 and rel.relname = $2
order by c.conname
limit $3
