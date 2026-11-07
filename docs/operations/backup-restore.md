# Data, persistence, backup and restore

MindGraph-OSS v0.1 keeps everything in three Docker volumes. This page says
what survives which command, how to clean up safely, and how to back up and
restore. The backup and restore procedure below was tested end to end on
2026-11-07 (Podman 6.1, `podman compose`): back up, `down -v`, restore, and
all graphs, demo checks and the smoke test came back intact.

## Where the data lives

| Volume (`docker volume ls`) | Service | Contents |
|---|---|---|
| `mindgraph_postgres-data` | postgres | metadata: graphs, ontology versions, data sources, mappings, jobs, entity resolution scans and decisions |
| `mindgraph_hugegraph-data` | hugegraph | the graph itself: entities and relationships (RocksDB), plus the HugeGraph server files and its "initialised" marker |
| `mindgraph_mindgraph-data` | core | uploaded CSV and JSON files (never served over HTTP) |

`mindgraph` is the Compose project name from `docker-compose.yml`. If you run
with `-p <name>`, the volumes are `<name>_postgres-data` and so on.

PostgreSQL and HugeGraph belong together: metadata in one refers to graph data
in the other. Always back up and restore all three volumes as a set.

## What survives what

| Command | Containers | Data |
|---|---|---|
| `docker compose stop` / `start` | kept | kept |
| `docker compose down` then `up -d` | recreated | **kept** |
| `docker compose up -d --build` (new images) | recreated | kept |
| restarting Docker or the machine | restarted (`restart: unless-stopped`) | kept |
| `docker compose down -v` | removed | **deleted: every graph, upload and decision** |
| `docker volume rm mindgraph_…` / `docker volume prune` | n/a | deleted |

`down` without `-v` is always safe. `scripts/smoke-stack.sh` checks this on
every CI run: it restarts the stack with `down` + `up -d` and verifies that
the demo graph is still there.

## Cleaning up safely

- **Stop MindGraph, keep the data:** `docker compose down`.
- **Free disk space without losing data:** `docker image prune` removes
  unused images only. Avoid `docker system prune --volumes` and
  `docker volume prune`: they delete unused volumes, including MindGraph's
  while the stack is down.
- **Remove a demo graph:** v0.1 has no graph deletion (UI or API). Demo
  graphs are small; leave them, or start over with the next item.
- **Start over completely:** back up first if anything is worth keeping,
  then `docker compose down -v` and `docker compose up -d`. This is also the
  way to apply a changed PostgreSQL user or password (PostgreSQL reads them
  only when its volume is created).
- **Test stacks:** `scripts/smoke-stack.sh` runs under its own project name
  (`mindgraph-smoke`) and removes only its own containers and volumes.

## Back up

Run from the repository folder. It stops the services that write, so take
the backup when nobody is using MindGraph. The examples use
`postgres:17-alpine` for `tar`, because Compose has already pulled it.

```bash
mkdir -p backup

# 1. Stop the writers: no ingestion, resolution decisions or uploads now.
docker compose stop web core

# 2. Metadata (PostgreSQL keeps running for the dump).
docker compose exec -T postgres pg_dump -U mindgraph -d mindgraph -Fc > backup/metadata.dump

# 3. Graph data and uploads (HugeGraph stopped so RocksDB is consistent).
docker compose stop hugegraph
for v in hugegraph-data mindgraph-data; do
  docker run --rm -v mindgraph_$v:/volume:ro -v "$PWD/backup:/backup" postgres:17-alpine \
    tar czf /backup/$v.tar.gz -C /volume .
done

# 4. Start again.
docker compose up -d
```

Use your own PostgreSQL user and database if you changed
`MINDGRAPH_POSTGRES_USER` or `MINDGRAPH_POSTGRES_DB`.

Size to expect: the demo stack produced a 59 KB metadata dump, a 4 KB uploads
archive and a 362 MB HugeGraph archive. The HugeGraph volume contains the
whole HugeGraph server directory (about 380 MB of libraries) as well as the
RocksDB data, because that is the volume the image declares.

## Restore

Restore into the same MindGraph version and the same HugeGraph image
(`hugegraph/hugegraph:1.7.0`); the HugeGraph archive includes server files of
that version.

```bash
# 1. Remove the current containers and volumes (this deletes current data).
docker compose down -v

# 2. Create empty containers and volumes, then unpack the archives into them.
docker compose up --no-start
for v in hugegraph-data mindgraph-data; do
  docker run --rm -v mindgraph_$v:/volume -v "$PWD/backup:/backup:ro" postgres:17-alpine \
    sh -c "rm -rf /volume/* /volume/.[!.]* 2>/dev/null; tar xzf /backup/$v.tar.gz -C /volume"
done

# 3. Restore the metadata before Core starts.
docker compose start postgres
until docker compose exec -T postgres pg_isready -U mindgraph -d mindgraph; do sleep 1; done
docker compose exec -T postgres pg_restore -U mindgraph -d mindgraph --clean --if-exists --no-owner \
  < backup/metadata.dump

# 4. Start everything and check.
docker compose up -d
curl -fsS http://localhost:8080/ready     # or your MINDGRAPH_CORE_PORT
```

With Podman, replace `docker compose` with `podman compose` and `docker run`
with `podman run`.

## Not covered in v0.1

- No online (hot) backup and no point-in-time recovery: the procedure stops
  writes.
- No automated or scheduled backups and no backup encryption; protect the
  `backup/` folder yourself, it contains your data.
- No migration path between HugeGraph versions; a different image version
  needs a fresh volume and a re-import.
