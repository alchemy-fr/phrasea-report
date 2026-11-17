# Phrasea Report API

Log-collection service for [Phrasea](https://github.com/alchemy-fr/phrasea): a small Rust
HTTP API that receives application events and stores them in PostgreSQL.

The Phrasea applications send events through
[`alchemy/report-bundle`](https://github.com/alchemy-fr/phrasea/tree/master/lib/php/report-bundle)
/ [`alchemy/report-sdk`](https://github.com/alchemy-fr/phrasea/tree/master/lib/php/report-sdk),
which post to this service (fire-and-forget).

## API

| Endpoint | Method | Description |
|---|---|---|
| `/log` | POST | JSON event (`appName`, `appId`, `action`, `item`, `user`, `eventDate`, `payload`); queued then inserted into the `logs` table |

`POST /log` answers `true` (200) once the event is queued, `Invalid JSON` (400) when the
body cannot be decoded, `Log queue full` (503) when the queue is full. Absent text fields
are stored as empty strings, an absent `eventDate` (Unix timestamp, in seconds) as 0, and
unknown fields are ignored.

Events are buffered in memory (queue of 1000) and written one at a time by a single
worker. When the database connection is lost, the worker reconnects (every 5 seconds,
giving up and exiting after 10 minutes) and retries the event. An event the database
refuses for another reason is appended to the failed logs file, which is replayed at
startup and after each reconnection. The schema (`src/structure.sql`) is created on each
connection if missing.

On SIGTERM or SIGINT the server stops accepting requests and the worker writes the
queued events before exiting.

## Configuration

| Env var | Description |
|---|---|
| `APP_ENV` | Only logged at startup (`prod` or dev mode) |
| `POSTGRES_HOST` | PostgreSQL host (default `localhost`) |
| `POSTGRES_PORT` | PostgreSQL port (default `5432`) |
| `POSTGRES_USER` | PostgreSQL user |
| `POSTGRES_PASSWORD` | PostgreSQL password |
| `POSTGRES_DATABASE` | Database name |
| `LISTEN_PORT` | HTTP port (default `80`) |
| `FAILED_LOGS_FILE` | Failed logs file (default `failed_logs.jsonl`, in `/srv/app` in the image) |

The image runs as user `app` (uid 1000); the binary carries `CAP_NET_BIND_SERVICE` to
listen on port 80.

## Run

The image is published as `ps-report-api` to the registry configured in CI:

```bash
docker run --rm -p 8080:80 -e APP_ENV=prod -e POSTGRES_HOST=... <registry>/ps-report-api:latest
```

Or build locally:

```bash
docker build -t report-api .
docker run --rm -p 8080:80 -e APP_ENV=prod -e POSTGRES_HOST=... report-api
```

Quick test:

```bash
curl -X POST http://localhost:8080/log -H 'Content-Type: application/json' \
  -d '{"appName": "databox", "action": "test", "eventDate": 1750000000, "payload": {}}'
```

## Development

Nothing needs to be installed on the host but Docker: `bin/cargo.sh` runs cargo in the
`rust-dev` stage of the Dockerfile (with rustfmt and clippy).

```bash
bin/cargo.sh test
bin/cargo.sh clippy --all-targets -- -D warnings
bin/cargo.sh fmt
```

## CI

Every push and pull request checks the formatting, runs clippy and the unit tests, then
builds the Docker image. Pushes to `main` publish
`<ECR_REGISTRY>/ps-report-api:latest` (plus the commit SHA); any git tag publishes the
corresponding image tag (semver tags also publish the stripped version). Publishing
uses the `ECR_REGISTRY` repository variable and the `AWS_ACCESS_KEY_ID` /
`AWS_SECRET_ACCESS_KEY` repository secrets; pull requests only build, without pushing.
