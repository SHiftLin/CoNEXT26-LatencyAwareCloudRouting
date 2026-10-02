# Database connection for processing tools

The main simulator's `--user` mode does not need a database. The traceroute and
link-processing tools that use `PostgresClient` read their PostgreSQL connection
settings from `BGP_SIM_DATABASE_URL` at runtime. Set it to a PostgreSQL URL or
keyword/value connection string, for example:

```sh
export BGP_SIM_DATABASE_URL='postgresql://USER:PASSWORD@HOST:5432/DATABASE'
```

Set the real value in your local environment or secret manager; do not add it
to source files or example inputs. If the variable is unset, connection setup
fails with an explanatory error.
