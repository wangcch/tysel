# SQLite worker

This service uses the native SQLite capability as a persistent counter.

From this example directory:

```sh
tysel config validate
tysel check
tysel run
```

Each request increments the selected key:

```sh
curl 'http://127.0.0.1:3000/?key=visits'
curl 'http://127.0.0.1:3000/?key=visits'
```

While using project commands from this directory, data is stored at
`data/tysel.db`. A packaged executable resolves the same relative path from its
process working directory. SQLite is available to the trusted `service`
profile. Use parameterized statements, set a stable production working
directory, keep the database on persistent storage, and include it in backup
and recovery procedures.
