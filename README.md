# Phrasea Report API

Log-collection service for [Phrasea](https://github.com/alchemy-fr/phrasea): a small Go
HTTP API that receives application events and stores them in PostgreSQL.

The Phrasea applications send events through
[`alchemy/report-bundle`](https://github.com/alchemy-fr/phrasea/tree/master/lib/php/report-bundle)
/ [`alchemy/report-sdk`](https://github.com/alchemy-fr/phrasea/tree/master/lib/php/report-sdk),
which post to this service (fire-and-forget).

## API

| Endpoint | Method | Description |
|---|---|---|
| `/log` | POST | JSON event (`appName`, `appId`, `action`, `item`, `user`, `eventDate`, `payload`); queued then inserted into the `logs` table |

Events are buffered in memory (queue of 1000) and written asynchronously; on database
outage they are appended to `failed_logs.jsonl` and replayed after reconnection. The
schema is created automatically at startup.

## Configuration

| Env var | Description |
|---|---|
| `APP_ENV` | `prod` runs the compiled binary; anything else runs with live-reload (`fresh`) |
| `POSTGRES_HOST` | PostgreSQL host |
| `POSTGRES_PORT` | PostgreSQL port |
| `POSTGRES_USER` | PostgreSQL user |
| `POSTGRES_PASSWORD` | PostgreSQL password |
| `POSTGRES_DATABASE` | Database name |

The server listens on port `80`.

## Run

The image is published to the public AWS ECR registry:

```bash
docker run --rm -p 8080:80 -e APP_ENV=prod -e POSTGRES_HOST=... public.ecr.aws/b2s9z7l1/ps-report-api:latest
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

## CI

Every push and pull request builds the Docker image. Pushes to `main` publish
`public.ecr.aws/b2s9z7l1/ps-report-api:latest` (plus the commit SHA); `v*` tags publish
the corresponding semver tag. Publishing uses the `AWS_ACCESS_KEY_ID` /
`AWS_SECRET_ACCESS_KEY` repository secrets; pull requests only build, without pushing.
