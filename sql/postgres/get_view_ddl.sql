select pg_get_viewdef(format('%I.%I', $1::text, $2::text)::regclass, true) as view_definition
