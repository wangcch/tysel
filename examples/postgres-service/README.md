# Postgres service

This service demonstrates a named, read-write Postgres grant. The manifest
contains only the grant name and mode; the connection URL stays in the host
environment.

From this example directory:

```sh
export TYSEL_POSTGRES_MAIN='postgres://user:password@127.0.0.1:5432/database'
tysel config validate
tysel check
tysel run
```

Call `http://127.0.0.1:3000/`. The handler creates a table, upserts one row,
queries it with parameters, and returns the result.

For read-only applications, change the grant to `main:read-only`. Never place a
database URL in the TOML or JSON manifest; a grant named `main` resolves only from
`TYSEL_POSTGRES_MAIN`.
