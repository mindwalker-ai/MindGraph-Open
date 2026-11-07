//! `mindgraph demo <scenario>`: loads a demo scenario through MindGraph
//! Core's public REST API, then checks that the graph was built correctly.
//!
//! A scenario is a directory under `examples/` with a `demo.json` manifest:
//! the graph name, the ontology file, the datasets with their M4 mappings
//! (in load order) and the expectations to verify. The loader never touches
//! PostgreSQL or HugeGraph directly: every step is an ordinary API call, so a
//! demo exercises the same path as a user.
//!
//! ```text
//! wait for /ready ─► find or create the graph ─► ontology ─► for each dataset:
//!   upload ─► analyse (job) ─► ingest with the mapping (job)
//! ─► verify: entity counts, exact searches, neighbours, shortest paths,
//!    entity resolution candidates ─► print the graph id and Explorer URL
//! ```
//!
//! **Re-running** a demo reuses a graph with the demo's exact name: it is
//! verified, not loaded again, so nothing is duplicated. `--new` creates
//! another copy with a timestamped name. Unrelated graphs are never changed.
//! Every wait is bounded; any failed step exits non-zero with the API's error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

const HELP: &str = "\
Load a demo scenario through the MindGraph Core REST API

Usage: mindgraph demo <SCENARIO> [OPTIONS]
       mindgraph demo --list

Scenarios are the directories of the examples folder that hold a demo.json
(fraud, cyber, knowledge-graph).

Options:
  --api <URL>        Core API base URL [env: MINDGRAPH_API_URL]
                     [default: http://127.0.0.1:8080]
  --web <URL>        Web app URL used in the printed links [env: MINDGRAPH_WEB_URL]
                     [default: http://localhost:3000]
  --examples <DIR>   The examples folder [env: MINDGRAPH_EXAMPLES_DIR] [default: examples]
  --timeout <SECS>   Longest wait for Core and for each job [default: 300]
  --new              Create another copy even if the demo graph already exists
  --list             List the available scenarios
  -h, --help         Print help

Re-running a demo reuses and verifies the existing demo graph; it never
overwrites or deletes data.";

/// A scenario manifest (`examples/<scenario>/demo.json`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    title: String,
    summary: String,
    graph: GraphSpec,
    ontology: String,
    sources: Vec<Source>,
    expect: Expect,
    /// The entity the printed Explorer link opens.
    explore: EntityRef,
    #[serde(default)]
    resolution: Option<ResolutionCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphSpec {
    name: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    file: String,
    mapping: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    /// Entities per type (exact count, at most 1,000 per type).
    #[serde(default)]
    entities: BTreeMap<String, u64>,
    /// Relationships created by all ingestion jobs (checked on a fresh load).
    #[serde(default)]
    relationships_created: Option<u64>,
    /// Entities that must exist with these property values.
    #[serde(default)]
    find: Vec<FindCheck>,
    #[serde(default)]
    neighbors: Vec<NeighborCheck>,
    #[serde(default)]
    paths: Vec<PathCheck>,
}

/// Identifies one entity by an exact property value.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityRef {
    entity_type: String,
    property: String,
    value: Value,
}

impl EntityRef {
    fn describe(&self) -> String {
        format!("{} {}={}", self.entity_type, self.property, self.value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindCheck {
    entity: EntityRef,
    #[serde(default)]
    properties: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeighborCheck {
    of: EntityRef,
    #[serde(default = "both")]
    direction: String,
    relationship_type: String,
    count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathCheck {
    from: EntityRef,
    to: EntityRef,
    hops: u64,
    #[serde(default)]
    via: Vec<String>,
    #[serde(default = "default_depth")]
    max_depth: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionCheck {
    entity_type: String,
    rules: Vec<Value>,
    /// Candidates (any state) that must exist after the scan.
    min_candidates: u64,
}

fn both() -> String {
    "both".to_owned()
}

const fn default_depth() -> usize {
    6
}

#[derive(Debug)]
struct Options {
    scenario: String,
    api: String,
    web: String,
    examples: PathBuf,
    timeout: Duration,
    new: bool,
}

/// Why a demo run failed.
#[derive(Debug)]
enum Failure {
    /// Bad arguments or files (exit code 2).
    Usage(String),
    /// A step failed (exit code 1).
    Step(String),
}

type Outcome<T> = Result<T, Failure>;

fn step<T>(message: impl Into<String>) -> Outcome<T> {
    Err(Failure::Step(message.into()))
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn parse(args: Vec<String>) -> Outcome<Option<Options>> {
    let mut scenario = None;
    let mut api = env_or("MINDGRAPH_API_URL", "http://127.0.0.1:8080");
    let mut web = env_or("MINDGRAPH_WEB_URL", "http://localhost:3000");
    let mut examples = PathBuf::from(env_or("MINDGRAPH_EXAMPLES_DIR", "examples"));
    let mut timeout = Duration::from_secs(300);
    let mut new = false;
    let mut list = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(None);
            }
            "--api" => api = value("--api")?,
            "--web" => web = value("--web")?,
            "--examples" => examples = PathBuf::from(value("--examples")?),
            "--timeout" => {
                let raw = value("--timeout")?;
                let seconds: u64 = raw
                    .parse()
                    .ok()
                    .filter(|s| (1..=3600).contains(s))
                    .ok_or_else(|| Failure::Usage("--timeout must be 1-3600 seconds".into()))?;
                timeout = Duration::from_secs(seconds);
            }
            "--new" => new = true,
            "--list" => list = true,
            flag if flag.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown option `{flag}`")));
            }
            name if scenario.is_none() => scenario = Some(name.to_owned()),
            extra => return Err(Failure::Usage(format!("unexpected argument `{extra}`"))),
        }
    }
    if list {
        print_scenarios(&examples)?;
        return Ok(None);
    }
    let scenario = scenario.ok_or_else(|| Failure::Usage("missing <SCENARIO>".into()))?;
    if scenario.contains(['/', '\\']) || scenario.starts_with('.') {
        return Err(Failure::Usage(format!(
            "`{scenario}` is not a scenario name"
        )));
    }
    Ok(Some(Options {
        scenario,
        api: api.trim_end_matches('/').to_owned(),
        web: web.trim_end_matches('/').to_owned(),
        examples,
        timeout,
        new,
    }))
}

fn scenarios(examples: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(examples)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().join("demo.json").is_file())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn print_scenarios(examples: &Path) -> Outcome<()> {
    let names = scenarios(examples);
    if names.is_empty() {
        return Err(Failure::Usage(format!(
            "no scenarios in `{}` (run from the repository root or pass --examples)",
            examples.display()
        )));
    }
    for name in names {
        let manifest = load_manifest(&examples.join(&name))?;
        println!("{name:<16} {}: {}", manifest.title, manifest.summary);
    }
    Ok(())
}

fn read(path: &Path) -> Outcome<Vec<u8>> {
    std::fs::read(path)
        .map_err(|err| Failure::Usage(format!("cannot read {}: {err}", path.display())))
}

fn read_json(path: &Path) -> Outcome<Value> {
    serde_json::from_slice(&read(path)?)
        .map_err(|err| Failure::Usage(format!("{} is not valid JSON: {err}", path.display())))
}

fn load_manifest(dir: &Path) -> Outcome<Manifest> {
    let path = dir.join("demo.json");
    serde_json::from_slice(&read(&path)?)
        .map_err(|err| Failure::Usage(format!("{} is not a valid manifest: {err}", path.display())))
}

/// A thin client for the endpoints the loader needs.
struct Api {
    http: Client,
    base: String,
}

impl Api {
    fn new(base: &str) -> Outcome<Self> {
        let http = Client::builder()
            .user_agent(concat!("mindgraph-demo/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|err| Failure::Step(format!("cannot build the HTTP client: {err}")))?;
        Ok(Self {
            http,
            base: base.to_owned(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    async fn send(
        &self,
        what: &str,
        request: reqwest::RequestBuilder,
        expected: &[StatusCode],
    ) -> Outcome<Value> {
        let response = request.send().await.map_err(|err| {
            Failure::Step(format!(
                "{what}: Core is not reachable at {} ({err})",
                self.base
            ))
        })?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if expected.contains(&status) {
            return Ok(body);
        }
        let error = &body["error"];
        step(format!(
            "{what} failed: HTTP {status} {} {} (request id {})",
            error["code"].as_str().unwrap_or("UNKNOWN"),
            error["message"].as_str().unwrap_or("no error message"),
            error["request_id"].as_str().unwrap_or("-"),
        ))
    }

    async fn get(&self, what: &str, path: &str) -> Outcome<Value> {
        self.send(what, self.http.get(self.url(path)), &[StatusCode::OK])
            .await
    }

    async fn post(
        &self,
        what: &str,
        path: &str,
        body: &Value,
        expected: &[StatusCode],
    ) -> Outcome<Value> {
        self.send(what, self.http.post(self.url(path)).json(body), expected)
            .await
    }

    /// Waits until `/ready` answers (Core starts after PostgreSQL and HugeGraph).
    async fn wait_ready(&self, timeout: Duration) -> Outcome<()> {
        let deadline = Instant::now() + timeout;
        let mut announced = false;
        loop {
            let ready = self
                .http
                .get(format!("{}/ready", self.base))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            if ready {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return step(format!(
                    "MindGraph Core at {} was not ready within {} s (is the stack running? `docker compose up -d`)",
                    self.base,
                    timeout.as_secs()
                ));
            }
            if !announced {
                println!("Waiting for MindGraph Core at {} ...", self.base);
                announced = true;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Polls a job until it completes; a failed job is an error.
    async fn wait_job(&self, what: &str, job_id: &str, timeout: Duration) -> Outcome<Value> {
        if job_id.is_empty() {
            return step(format!("{what}: Core returned no job id"));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let job = self.get(what, &format!("/jobs/{job_id}")).await?;
            match job["status"].as_str() {
                Some("completed") => return Ok(job),
                Some("failed" | "cancelled") => {
                    return step(format!(
                        "{what} failed: {} {}",
                        job["error"]["code"].as_str().unwrap_or("JOB_FAILED"),
                        job["error"]["message"].as_str().unwrap_or("no details")
                    ));
                }
                _ if Instant::now() >= deadline => {
                    return step(format!(
                        "{what} did not finish within {} s (job {job_id})",
                        timeout.as_secs()
                    ));
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// The entities matching one exact property value (at most `limit`).
    async fn find(&self, graph: &str, entity: &EntityRef, limit: usize) -> Outcome<Vec<Value>> {
        let body = json!({
            "entity_type": entity.entity_type,
            "property_filters": { entity.property.clone(): entity.value },
            "limit": limit,
        });
        let found = self
            .post(
                &format!("searching {}", entity.describe()),
                &format!("/graphs/{graph}/entities/search"),
                &body,
                &[StatusCode::OK],
            )
            .await?;
        Ok(found["data"].as_array().cloned().unwrap_or_default())
    }

    async fn find_one(&self, graph: &str, entity: &EntityRef) -> Outcome<Value> {
        let mut found = self.find(graph, entity, 2).await?;
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => step(format!(
                "expected entity {} was not found",
                entity.describe()
            )),
            _ => step(format!(
                "{} matches more than one entity",
                entity.describe()
            )),
        }
    }
}

fn id_of(entity: &Value) -> Outcome<String> {
    entity["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Failure::Step("Core returned an entity without an id".into()))
}

async fn load(
    api: &Api,
    dir: &Path,
    manifest: &Manifest,
    graph: &str,
    timeout: Duration,
) -> Outcome<u64> {
    let ontology = read_json(&dir.join(&manifest.ontology))?;
    let created = api
        .post(
            "creating the ontology",
            &format!("/graphs/{graph}/ontology"),
            &ontology,
            &[StatusCode::OK, StatusCode::CREATED],
        )
        .await?;
    let types: Vec<&str> = created["entity_types"]
        .as_array()
        .map(|types| types.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    println!("  ontology v{}: {}", created["version"], types.join(", "));
    let mut relationships = 0;
    for source in &manifest.sources {
        let bytes = read(&dir.join(&source.file))?;
        let mapping = read_json(&dir.join(&source.mapping))?;
        let mime = if source.file.ends_with(".json") {
            "application/json"
        } else {
            "text/csv"
        };
        let part = Part::bytes(bytes)
            .file_name(source.file.clone())
            .mime_str(mime)
            .map_err(|err| Failure::Step(format!("cannot prepare {}: {err}", source.file)))?;
        let uploaded = api
            .send(
                &format!("uploading {}", source.file),
                api.http
                    .post(api.url(&format!("/graphs/{graph}/data-sources")))
                    .multipart(Form::new().part("file", part)),
                &[StatusCode::CREATED, StatusCode::OK],
            )
            .await?;
        let data_source = uploaded["id"].as_str().unwrap_or_default().to_owned();
        let analysis = api
            .post(
                &format!("analysing {}", source.file),
                &format!("/graphs/{graph}/data-sources/{data_source}/analyze"),
                &json!({}),
                &[StatusCode::ACCEPTED, StatusCode::OK],
            )
            .await?;
        let job = analysis["job_id"].as_str().unwrap_or_default().to_owned();
        api.wait_job(&format!("analysing {}", source.file), &job, timeout)
            .await?;
        let ingestion = api
            .send(
                &format!("ingesting {}", source.file),
                api.http
                    .post(api.url(&format!(
                        "/graphs/{graph}/data-sources/{data_source}/ingestions"
                    )))
                    .header("Idempotency-Key", uuid::Uuid::now_v7().to_string())
                    .json(&mapping),
                &[StatusCode::ACCEPTED, StatusCode::OK, StatusCode::CREATED],
            )
            .await?;
        let job = ingestion["job_id"]
            .as_str()
            .or_else(|| ingestion["id"].as_str())
            .unwrap_or_default()
            .to_owned();
        let done = api
            .wait_job(&format!("ingesting {}", source.file), &job, timeout)
            .await?;
        let result = &done["result"];
        relationships += result["relationships_created"].as_u64().unwrap_or(0);
        println!(
            "  {:<26} {} records · {} entities created · {} matched · {} relationships",
            source.file,
            result["records_processed"],
            result["entities_created"],
            result["entities_matched"],
            result["relationships_created"],
        );
    }
    Ok(relationships)
}

async fn verify(api: &Api, manifest: &Manifest, graph: &str) -> Outcome<()> {
    let expect = &manifest.expect;
    for (entity_type, expected) in &expect.entities {
        let body = json!({ "entity_type": entity_type, "limit": 1000 });
        let found = api
            .post(
                &format!("counting {entity_type} entities"),
                &format!("/graphs/{graph}/entities/search"),
                &body,
                &[StatusCode::OK],
            )
            .await?;
        let count = found["count"].as_u64().unwrap_or(0);
        if count != *expected {
            return step(format!(
                "expected {expected} {entity_type} entities, found {count}"
            ));
        }
    }
    let counts: Vec<String> = expect
        .entities
        .iter()
        .map(|(entity_type, count)| format!("{count} {entity_type}"))
        .collect();
    println!("  entities: {}", counts.join(", "));
    for check in &expect.find {
        let entity = api.find_one(graph, &check.entity).await?;
        for (key, value) in &check.properties {
            if &entity["properties"][key] != value {
                return step(format!(
                    "{} has {key}={}, expected {value}",
                    check.entity.describe(),
                    entity["properties"][key]
                ));
            }
        }
    }
    for check in &expect.neighbors {
        let entity = id_of(&api.find_one(graph, &check.of).await?)?;
        let neighbors = api
            .get(
                &format!("reading the neighbours of {}", check.of.describe()),
                &format!(
                    "/graphs/{graph}/entities/{entity}/neighbors?direction={}&relationship_type={}&limit=100",
                    check.direction, check.relationship_type
                ),
            )
            .await?;
        let count = neighbors["relationships"].as_array().map_or(0, Vec::len);
        if count != check.count {
            return step(format!(
                "expected {} {} relationships of {}, found {count}",
                check.count,
                check.relationship_type,
                check.of.describe()
            ));
        }
    }
    for check in &expect.paths {
        let source = id_of(&api.find_one(graph, &check.from).await?)?;
        let target = id_of(&api.find_one(graph, &check.to).await?)?;
        let path = api
            .post(
                "finding a path",
                &format!("/graphs/{graph}/paths"),
                &json!({
                    "source_id": source,
                    "target_id": target,
                    "direction": "both",
                    "max_depth": check.max_depth,
                }),
                &[StatusCode::OK],
            )
            .await?;
        let found = path["found"] == json!(true);
        let hops = path["hops"].as_u64().unwrap_or(0);
        if !found || hops != check.hops {
            return step(format!(
                "expected a {}-hop path from {} to {}, found {}",
                check.hops,
                check.from.describe(),
                check.to.describe(),
                if found {
                    format!("{hops} hops")
                } else {
                    "none".into()
                }
            ));
        }
        let via: Vec<&str> = path["relationships"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter_map(|r| r["relationship_type"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        if !check.via.is_empty() && via != check.via {
            return step(format!(
                "the path goes via {via:?}, expected {:?}",
                check.via
            ));
        }
        println!(
            "  path {} → {}: {hops} hops via {}",
            check.from.describe(),
            check.to.describe(),
            via.join(" · ")
        );
    }
    Ok(())
}

/// Candidates in any state (pending, accepted, rejected) and the pending count.
async fn candidate_counts(api: &Api, graph: &str) -> Outcome<(u64, Value)> {
    let counts = api
        .get(
            "counting resolution candidates",
            &format!("/graphs/{graph}/resolution/candidates?status=all&limit=1"),
        )
        .await?["counts"]
        .clone();
    let total = ["pending", "accepted", "rejected"]
        .iter()
        .map(|status| counts[status].as_u64().unwrap_or(0))
        .sum();
    Ok((total, counts["pending"].clone()))
}

/// Makes sure the graph has entity resolution candidates to review. A scan
/// only runs when there are too few, so re-running a demo adds no scans and
/// never touches decisions already taken.
async fn resolution(api: &Api, check: &ResolutionCheck, graph: &str) -> Outcome<()> {
    let (mut total, mut pending) = candidate_counts(api, graph).await?;
    if total < check.min_candidates {
        let scan = api
            .post(
                "running an entity resolution scan",
                &format!("/graphs/{graph}/resolution/scans"),
                &json!({ "entity_type": check.entity_type, "rules": check.rules }),
                &[StatusCode::CREATED],
            )
            .await?;
        println!(
            "  entity resolution scan: {} entities examined, {} candidates found",
            scan["entities_examined"], scan["candidates_found"]
        );
        (total, pending) = candidate_counts(api, graph).await?;
    }
    if total < check.min_candidates {
        return step(format!(
            "expected at least {} entity resolution candidates, found {total}",
            check.min_candidates
        ));
    }
    println!("  entity resolution: {total} candidates, {pending} pending review");
    Ok(())
}

async fn run_demo(options: Options) -> Outcome<()> {
    let dir = options.examples.join(&options.scenario);
    if !dir.join("demo.json").is_file() {
        let known = scenarios(&options.examples);
        let hint = if known.is_empty() {
            format!(
                " (no scenarios in `{}`; pass --examples)",
                options.examples.display()
            )
        } else {
            format!("; available: {}", known.join(", "))
        };
        return Err(Failure::Usage(format!(
            "unknown scenario `{}`{hint}",
            options.scenario
        )));
    }
    let manifest = load_manifest(&dir)?;
    let api = Api::new(&options.api)?;
    api.wait_ready(options.timeout).await?;
    println!("{}: {}", manifest.title, manifest.summary);

    let graphs = api.get("listing graphs", "/graphs?limit=100").await?;
    let existing = graphs["data"].as_array().and_then(|all| {
        all.iter()
            .find(|g| g["name"].as_str() == Some(manifest.graph.name.as_str()))
            .and_then(|g| g["id"].as_str().map(str::to_owned))
    });

    let (graph, loaded_now) = match existing {
        Some(graph) if !options.new => {
            println!(
                "Found the existing demo graph `{}` ({graph}); verifying it instead of loading it again.",
                manifest.graph.name
            );
            (graph, false)
        }
        _ => {
            let name = if options.new {
                format!("{} · {}", manifest.graph.name, utc_now())
            } else {
                manifest.graph.name.clone()
            };
            let created = api
                .post(
                    "creating the graph",
                    "/graphs",
                    &json!({ "name": name, "description": manifest.graph.description }),
                    &[StatusCode::CREATED],
                )
                .await?;
            let graph = created["id"].as_str().unwrap_or_default().to_owned();
            println!("Created graph `{name}` ({graph}). Loading datasets:");
            let relationships = load(&api, &dir, &manifest, &graph, options.timeout).await?;
            if let Some(expected) = manifest.expect.relationships_created
                && relationships != expected
            {
                return step(format!(
                    "expected {expected} relationships to be created, the ingestion jobs created {relationships}"
                ));
            }
            (graph, true)
        }
    };

    println!("Verifying the graph:");
    verify(&api, &manifest, &graph)
        .await
        .map_err(|failure| match failure {
            Failure::Step(message) if !loaded_now => Failure::Step(format!(
                "{message}\nThe existing graph `{}` does not match this demo (perhaps an earlier load stopped part-way). \
                 Run again with --new to load a fresh copy; the existing graph is left untouched.",
                manifest.graph.name
            )),
            other => other,
        })?;
    if let Some(check) = &manifest.resolution {
        resolution(&api, check, &graph).await?;
    }
    let focus = id_of(&api.find_one(&graph, &manifest.explore).await?)?;
    println!();
    println!("Demo ready: {}", manifest.title);
    println!("  Graph ID           {graph}");
    println!(
        "  Graph Explorer     {}/graphs/{graph}/explore?entity={focus}",
        options.web
    );
    if manifest.resolution.is_some() {
        println!(
            "  Entity resolution  {}/graphs/{graph}/resolution",
            options.web
        );
    }
    println!(
        "  MCP setup          {}/graphs/{graph}/integrations/mcp",
        options.web
    );
    Ok(())
}

/// UTC time as `YYYY-MM-DD HH:MM:SS UTC` for unique demo copy names, without
/// a date crate (days since the epoch via the civil-from-days algorithm).
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Entry point of `mindgraph demo`.
pub fn run(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse(args.collect()) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(Failure::Usage(message) | Failure::Step(message)) => {
            eprintln!("mindgraph demo: {message}\n\nRun `mindgraph demo --help` for usage.");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("mindgraph demo: cannot start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run_demo(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("mindgraph demo: {message}");
            ExitCode::from(2)
        }
        Err(Failure::Step(message)) => {
            eprintln!("mindgraph demo: FAILED: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn options_parse_and_reject_bad_input() {
        let Ok(Some(options)) = parse(args(&["fraud", "--api", "http://core:8080/", "--new"]))
        else {
            panic!("valid arguments");
        };
        assert_eq!(options.scenario, "fraud");
        assert_eq!(options.api, "http://core:8080");
        assert!(options.new);
        for bad in [
            vec!["--timeout", "0", "fraud"],
            vec!["fraud", "extra"],
            vec!["../etc"],
            vec!["--bogus"],
            vec![],
        ] {
            assert!(
                matches!(parse(args(&bad)), Err(Failure::Usage(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn every_bundled_manifest_parses_and_names_existing_files() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let names = scenarios(&examples);
        assert_eq!(names, ["cyber", "fraud", "knowledge-graph"]);
        for name in names {
            let dir = examples.join(&name);
            let manifest = load_manifest(&dir).unwrap();
            assert!(dir.join(&manifest.ontology).is_file(), "{name}: ontology");
            for source in &manifest.sources {
                assert!(dir.join(&source.file).is_file(), "{name}: {}", source.file);
                assert!(
                    read_json(&dir.join(&source.mapping)).is_ok(),
                    "{name}: {}",
                    source.mapping
                );
            }
            assert!(!manifest.expect.entities.is_empty(), "{name}: expectations");
            assert!(!manifest.expect.paths.is_empty(), "{name}: a path to show");
        }
    }

    #[test]
    fn timestamps_are_utc() {
        let now = utc_now();
        assert_eq!(now.len(), "2026-11-06 12:00:00 UTC".len());
        assert!(now.starts_with("20") && now.ends_with("UTC"));
    }
}
