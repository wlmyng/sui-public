Sui indexer is an off-fullnode service to serve data from Sui protocol, including both data directly generated from chain and derivative data.

## Architecture
![enhanced_FN](https://user-images.githubusercontent.com/106119108/221022505-a1d873c6-60e2-45f1-b2aa-e50192c4dfbb.png)

## Steps to run locally
### Prerequisites
- install local [Postgres server](https://www.postgresql.org/download/). You can also `brew install postgresql@15` and then add the following to your `~/.zshrc` or `~/.zprofile`, etc:
```sh
export LDFLAGS="-L/opt/homebrew/opt/postgresql@15/lib"
export CPPFLAGS="-I/opt/homebrew/opt/postgresql@15/include"
export PATH="/opt/homebrew/opt/postgresql@15/bin:$PATH"
```
- make sure you have libpq installed: `brew install libpq`, and in your profile, add `export PATH="/opt/homebrew/opt/libpq/bin:$PATH"`. If this doesn't work, try `brew link --force libpq`.

- install Diesel CLI with `cargo install diesel_cli --no-default-features --features postgres`, refer to [Diesel Getting Started guide](https://diesel.rs/guides/getting-started) for more details
- [optional but handy] Postgres client like [Postico](https://eggerapps.at/postico2/), for local check, query execution etc.

### Start the Postgres Service

Postgres must run as a service in the background for other tools to communicate with.  If it was installed using homebrew, it can be started as a service with:

``` sh
brew services start postgresql@version
```

### Local Development(Recommended)

Use [sui-test-validator](../../crates/sui-test-validator/README.md)

### Running standalone indexer
1. DB setup, under `sui/crates/sui-indexer` run:
```sh
# an example DATABASE_URL is "postgres://postgres:postgres@localhost/gegao"
diesel setup --database-url="<DATABASE_URL>"
diesel database reset --database-url="<DATABASE_URL>"
```
Note that you'll need an existing database for the above to work. Replace `gegao` with the name of the database created.

2. Checkout to your target branch

For example, if you want to be on the DevNet branch
```sh
git fetch upstream devnet && git reset --hard upstream/devnet
```
3. Start indexer binary, under `sui/crates/sui-indexer` run:
- run indexer as a writer, which pulls data from fullnode and writes data to DB
```sh
# Change the RPC_CLIENT_URL to http://0.0.0.0:9000 to run indexer against local validator & fullnode
cargo run --bin sui-indexer -- --db-url "<DATABASE_URL>" --rpc-client-url "https://fullnode.devnet.sui.io:443" --fullnode-sync-worker --reset-db
```
- run indexer as a reader, which is a JSON RPC server with the [interface](https://docs.sui.io/sui-api-ref#suix_getallbalances)
```
cargo run --bin sui-indexer -- --db-url "<DATABASE_URL>" --rpc-client-url "https://fullnode.devnet.sui.io:443" --rpc-server-worker
```
More flags info can be found in this [file](https://github.com/MystenLabs/sui/blob/main/crates/sui-indexer/src/lib.rs#L83-L123).
### DB reset
Run this command under `sui/crates/sui-indexer`, which will wipe DB; In case of schema changes in `.sql` files, this will also update corresponding `schema.rs` file.
```sh
diesel database reset --database-url="<DATABASE_URL>"
```

## Steps to run locally (TiDB)

### Prerequisites

1. Install TiDB

``` sh
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh
```

2. Install a compatible version of MySQL (At the time of writing, this is MySQL 8.0 -- note that 8.3 is incompatible).

``` sh
brew install mysql@8.0
```

3. Install a version of `diesel_cli` that supports MySQL (and probably also Postgres). This version of the CLI needs to be built against the version of MySQL that was installed in the previous step (compatible with the local installation of TiDB, 8.0.37 at time of writing).

``` sh
MYSQLCLIENT_LIB_DIR=/opt/homebrew/Cellar/mysql@8.0/8.0.37/lib/ cargo install diesel_cli --no-default-features --features postgres --features mysql --force
```

### Run the indexer

1.Run TiDB

```sh
tiup playground
```

2.Verify tidb is running by connecting to it using the mysql client, create database `test`

```sh
mysql --comments --host 127.0.0.1 --port 4000 -u root
create database test;
```

3.DB setup, under `sui/crates/sui-indexer` run:

```sh
# an example DATABASE_URL is "mysql://root:password@127.0.0.1:4000/test"
diesel setup --database-url="<DATABASE_URL> --migration-dir='migrations/mysql'"
diesel database reset --database-url="<DATABASE_URL> --migration-dir='migrations/mysql'"
```

Note that you'll need an existing database for the above to work. Replace `test` with the name of the database created.
4. run indexer as a writer, which pulls data from fullnode and writes data to DB

```sh
# Change the RPC_CLIENT_URL to http://0.0.0.0:9000 to run indexer against local validator & fullnode
cargo run --bin sui-indexer --features mysql-feature --no-default-features -- --db-url "<DATABASE_URL>" --rpc-client-url "https://fullnode.devnet.sui.io:443" --fullnode-sync-worker --reset-db
```

# Environment variables

```
#*************************************************************************************************#
# Rust
#*************************************************************************************************#

RUST_BACKTRACE=1
RUST_LOG=info

#*************************************************************************************************#
# Indexer Config
#*************************************************************************************************#

DATABASE_USERNAME=postgres
DATABASE_PASSWORD=
DATABASE_NAME=
DATABASE_URL=postgres://$(DATABASE_USERNAME}:${DATABASE_PASSWORD}@localhost/${DATABASE_NAME}

RPC_CLIENT_URL=
CLIENT_METRIC_PORT=9185

#*************************************************************************************************#
# DB
#*************************************************************************************************#

DB_POOL_SIZE=100
DB_CONNECTION_TIMEOUT=3600
DB_STATEMENT_TIMEOUT=3600

#*************************************************************************************************#
# Indexer
#*************************************************************************************************#

DOWNLOAD_QUEUE_SIZE=200
INGESTION_READER_TIMEOUT_SECS=20
# Limit indexing parallelism on big checkpoints to avoid OOM,
# by limiting the total size of batch checkpoints to ~20MB.
# On testnet, most checkpoints are < 200KB, some can go up to 50MB.
CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT=20000000
CHECKPOINT_PROCESSING_BATCH_SIZE=100

#*************************************************************************************************#
# Objects Snapshot Processor
#*************************************************************************************************#

# The objects_snapshot table maintains a delayed snapshot of the objects table,
# controlled by object_snapshot_max_checkpoint_lag (max lag) and
# object_snapshot_min_checkpoint_lag (min lag). For instance, with a max lag of 900
# and a min lag of 300 checkpoints, the objects_snapshot table will lag behind the
# objects table by 300 to 900 checkpoints. The snapshot is updated when the lag
# exceeds the max lag threshold, and updates continue until the lag is reduced to
# the min lag threshold. Then, we have a consistent read range between
# latest_snapshot_cp and latest_cp based on objects_snapshot and objects_history,
# where the size of this range varies between the min and max lag values.

OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG=300
OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG=900

#*************************************************************************************************#
# Checkpoint Handler
#*************************************************************************************************#

CHECKPOINT_QUEUE_SIZE=100

#*************************************************************************************************#
# PG Indexer Store
#*************************************************************************************************#

PG_COMMIT_PARALLEL_CHUNK_SIZE=100
PG_COMMIT_OBJECTS_PARALLEL_CHUNK_SIZE=500
EPOCHS_TO_KEEP=20
SKIP_OBJECT_HISTORY=false
SKIP_OBJECT_SNAPSHOT=false

#*************************************************************************************************#
# Checkpoint Handler
#*************************************************************************************************#

CHECKPOINT_COMMIT_BATCH_SIZE=100

#*************************************************************************************************#
# Fetcher
#*************************************************************************************************#

CHECKPOINT_FETCH_INTERVAL_MS=500
```

# Useful SQL queries

```sql
CREATE OR REPLACE FUNCTION drop_partitions_below(table_prefix text, threshold bigint) RETURNS void AS $$
DECLARE
    partition_name text;
    drop_command text;
BEGIN
    FOR partition_name IN
        SELECT tablename
        FROM pg_tables
        WHERE schemaname = 'public'
        AND tablename ~ ('^' || table_prefix || '_partition_\\d+$')
        AND substring(tablename from ('\\d+$'))::bigint < threshold
    LOOP
        drop_command := 'DROP TABLE IF EXISTS public.' || partition_name || ' CASCADE';
        RAISE NOTICE 'Executing: %', drop_command; -- This line is optional for debugging
        EXECUTE drop_command;
    END LOOP;
END;
$$ LANGUAGE plpgsql;
```
Example:
```sql
SELECT drop_partitions_below('transactions', 50);

```
