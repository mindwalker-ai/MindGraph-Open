//! The ingestion engine: preflight, entity resolution and batched writes.
//!
//! 1. **Preflight** streams the whole file once and transforms every record
//!    without writing anything. Bad data fails the job before the first
//!    graph mutation.
//! 2. **Write** streams the file again, in batches. The identities of a
//!    batch that are not in the job-scoped cache are looked up with one
//!    `search_entities` call per entity type and chunk of identities (exact
//!    matching via `EntitySearch::with_any_of`), not one call per row. A type
//!    that was empty when the job started needs no lookups at all. Entities
//!    and relationships are then written with `create_entities` /
//!    `create_relationships`.
//!
//! Graph writes are not transactional across batches. If the store fails
//! mid-way, earlier batches remain; the error is reported, not hidden.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use mindgraph_domain::{
    DataSourceId, DataSourceKind, Entity, EntityId, GraphId, Relationship, RelationshipId, TypeName,
};
use mindgraph_graph::{EntitySearch, GraphStore};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::mapping::{
    EntityMode, PlannedEntity, RowPlan, ValidatedMapping, entity_identity_key, plan_row,
};
use crate::record::{Limits, for_each_record};
use crate::{IngestionError, SourceStore};

/// Counters reported by a finished ingestion. Contains no record data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionSummary {
    /// Records read and applied.
    pub records_processed: u64,
    /// New entities written.
    pub entities_created: u64,
    /// Distinct pre-existing entities reused.
    pub entities_matched: u64,
    /// New relationships written.
    pub relationships_created: u64,
}

/// Receives progress (`0.0..=1.0`) once per batch.
#[async_trait]
pub trait ProgressSink: Send + Sync {
    /// Reports progress.
    async fn report(&self, progress: f64);
}

/// What to ingest.
#[derive(Debug, Clone)]
pub struct IngestionRun {
    /// The target graph.
    pub graph_id: GraphId,
    /// The source file.
    pub data_source_id: DataSourceId,
    /// The source format.
    pub kind: DataSourceKind,
    /// The validated mapping.
    pub mapping: Arc<ValidatedMapping>,
    /// Reading limits.
    pub limits: Limits,
    /// Records per write batch (at least 1).
    pub batch_size: usize,
}

/// Fraction of progress reserved for the preflight pass.
const PREFLIGHT_SHARE: f64 = 0.1;

fn worker_failed(err: &tokio::task::JoinError) -> IngestionError {
    IngestionError::SourceStore {
        message: format!("the reader task failed: {err}"),
        source: None,
    }
}

/// Runs an ingestion. See the module documentation.
///
/// # Errors
///
/// Any data, mapping, storage or graph error; nothing is written if the
/// preflight pass fails.
pub async fn run_ingestion(
    run: &IngestionRun,
    graph: &dyn GraphStore,
    sources: &dyn SourceStore,
    progress: &dyn ProgressSink,
) -> Result<IngestionSummary, IngestionError> {
    let batch_size = run.batch_size.max(1);

    // Pass 1: preflight, no writes.
    let reader = sources.open(run.data_source_id).await?;
    let (mapping, limits, kind) = (Arc::clone(&run.mapping), run.limits, run.kind);
    let total = tokio::task::spawn_blocking(move || {
        for_each_record(kind, reader, &limits, &mut |number, record| {
            plan_row(&mapping, number, &record).map(drop)
        })
        .map(|stats| stats.record_count)
    })
    .await
    .map_err(|err| worker_failed(&err))??;
    progress.report(PREFLIGHT_SHARE).await;

    // Pass 2: stream plans from a reader thread into batched writes.
    let reader = sources.open(run.data_source_id).await?;
    let (sender, mut receiver) = mpsc::channel::<Result<RowPlan, IngestionError>>(batch_size);
    let mapping = Arc::clone(&run.mapping);
    let producer = tokio::task::spawn_blocking(move || {
        let result = for_each_record(kind, reader, &limits, &mut |number, record| {
            let plan = plan_row(&mapping, number, &record)?;
            sender
                .blocking_send(Ok(plan))
                .map_err(|_| IngestionError::SourceStore {
                    message: "ingestion stopped".to_owned(),
                    source: None,
                })
        });
        if let Err(err) = result {
            let _ = sender.blocking_send(Err(err));
        }
    });

    let mut writer = Writer::new(run, graph).await?;
    let mut batch = Vec::with_capacity(batch_size);
    while let Some(item) = receiver.recv().await {
        batch.push(item?);
        if batch.len() >= batch_size {
            writer
                .flush(std::mem::take(&mut batch), total, progress)
                .await?;
        }
    }
    writer.flush(batch, total, progress).await?;
    producer.await.map_err(|err| worker_failed(&err))?;
    Ok(writer.summary)
}

/// Identities resolved by one search: one below the result cap, so a full
/// result page always means some identity matched more than once (and the
/// truncation fallback runs). With the default batch size this is one
/// search per entity type per batch.
const LOOKUP_CHUNK: usize = EntitySearch::MAX_LIMIT - 1;

struct Writer<'a> {
    run: &'a IngestionRun,
    graph: &'a dyn GraphStore,
    /// Job-scoped identity cache: canonical identity → entity id.
    cache: HashMap<String, EntityId>,
    /// Types with no entities when the job started: lookups can be skipped.
    new_types: HashSet<TypeName>,
    /// Existing entities per identity, prefetched for the current batch.
    lookups: HashMap<String, Vec<EntityId>>,
    /// Job-scoped relationship de-duplication.
    seen_relationships: HashSet<String>,
    summary: IngestionSummary,
}

impl<'a> Writer<'a> {
    async fn new(run: &'a IngestionRun, graph: &'a dyn GraphStore) -> Result<Self, IngestionError> {
        let mut new_types = HashSet::new();
        for entity in &run.mapping.entities {
            if entity.mode == EntityMode::CreateOrMatch && !new_types.contains(&entity.entity_type)
            {
                let search = EntitySearch::new()
                    .with_entity_type(entity.entity_type.clone())
                    .with_limit(1)?;
                if graph
                    .search_entities(run.graph_id, &search)
                    .await?
                    .is_empty()
                {
                    new_types.insert(entity.entity_type.clone());
                }
            }
        }
        Ok(Self {
            run,
            graph,
            cache: HashMap::new(),
            new_types,
            lookups: HashMap::new(),
            seen_relationships: HashSet::new(),
            summary: IngestionSummary::default(),
        })
    }

    /// Whether entities of mapping `index` must be looked up in the graph.
    fn needs_lookup(&self, index: usize) -> bool {
        let mapping = &self.run.mapping.entities[index];
        mapping.mode == EntityMode::MatchOnly || !self.new_types.contains(&mapping.entity_type)
    }

    /// Looks up every uncached identity of the batch: one `search_entities`
    /// call per entity type and chunk of identities, not one per row.
    async fn prefetch(&mut self, batch: &[RowPlan]) -> Result<(), IngestionError> {
        self.lookups.clear();
        let mut wanted: BTreeMap<usize, Vec<&PlannedEntity>> = BTreeMap::new();
        let mut queued = HashSet::new();
        for plan in batch {
            for (index, planned) in plan.entities.iter().enumerate() {
                if !self.cache.contains_key(&planned.identity_key)
                    && self.needs_lookup(index)
                    && queued.insert(planned.identity_key.as_str())
                {
                    wanted.entry(index).or_default().push(planned);
                }
            }
        }
        for (index, identities) in wanted {
            let mapping = &self.run.mapping.entities[index];
            for chunk in identities.chunks(LOOKUP_CHUNK) {
                let alternatives = chunk
                    .iter()
                    .map(|planned| planned.identity.iter().cloned().collect())
                    .collect();
                let search = EntitySearch::new()
                    .with_entity_type(mapping.entity_type.clone())
                    .with_any_of(alternatives)?
                    .with_limit(EntitySearch::MAX_LIMIT)?;
                let found = self
                    .graph
                    .search_entities(self.run.graph_id, &search)
                    .await?;
                for planned in chunk {
                    self.lookups
                        .insert(planned.identity_key.clone(), Vec::new());
                }
                for entity in &found {
                    if let Some(ids) = entity_identity_key(mapping, entity)
                        .and_then(|key| self.lookups.get_mut(&key))
                    {
                        ids.push(entity.id());
                    }
                }
                if found.len() == EntitySearch::MAX_LIMIT {
                    // Truncated: matches may be missing. Confirm each identity
                    // that does not already have two (ambiguous) matches.
                    for planned in chunk {
                        if self.lookups[&planned.identity_key].len() < 2 {
                            let ids = self.lookup_one(index, planned).await?;
                            self.lookups.insert(planned.identity_key.clone(), ids);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn lookup_one(
        &self,
        index: usize,
        planned: &PlannedEntity,
    ) -> Result<Vec<EntityId>, IngestionError> {
        let mut search = EntitySearch::new()
            .with_entity_type(self.run.mapping.entities[index].entity_type.clone())
            .with_limit(2)?;
        for (key, value) in &planned.identity {
            search = search.with_property(key.clone(), value.clone());
        }
        Ok(self
            .graph
            .search_entities(self.run.graph_id, &search)
            .await?
            .iter()
            .map(Entity::id)
            .collect())
    }

    fn resolve(
        &mut self,
        index: usize,
        plan: &RowPlan,
        pending: &mut Vec<Entity>,
    ) -> Result<EntityId, IngestionError> {
        let planned = &plan.entities[index];
        if let Some(id) = self.cache.get(&planned.identity_key) {
            return Ok(*id);
        }
        let mapping = &self.run.mapping.entities[index];
        let existing = if self.needs_lookup(index) {
            self.lookups
                .get(&planned.identity_key)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let entity_type = mapping.entity_type.to_string();
        let id = match (existing.as_slice(), mapping.mode) {
            ([], EntityMode::CreateOrMatch) => {
                let id = EntityId::new();
                pending.push(Entity::new(
                    id,
                    self.run.graph_id,
                    mapping.entity_type.clone(),
                    planned.properties.clone(),
                ));
                self.summary.entities_created += 1;
                id
            }
            ([], EntityMode::MatchOnly) => {
                return Err(IngestionError::EntityNotFound {
                    record: plan.record,
                    entity_type,
                });
            }
            ([only], _) => {
                self.summary.entities_matched += 1;
                *only
            }
            _ => {
                return Err(IngestionError::AmbiguousEntity {
                    record: plan.record,
                    entity_type,
                });
            }
        };
        self.cache.insert(planned.identity_key.clone(), id);
        Ok(id)
    }

    async fn flush(
        &mut self,
        batch: Vec<RowPlan>,
        total: u64,
        progress: &dyn ProgressSink,
    ) -> Result<(), IngestionError> {
        if batch.is_empty() {
            return Ok(());
        }
        self.prefetch(&batch).await?;
        let mut entities = Vec::new();
        let mut relationships = Vec::new();
        for plan in &batch {
            let mut ids = Vec::with_capacity(plan.entities.len());
            for index in 0..plan.entities.len() {
                ids.push(self.resolve(index, plan, &mut entities)?);
            }
            for (mapping, properties) in self
                .run
                .mapping
                .relationships
                .iter()
                .zip(&plan.relationships)
            {
                let (source, target) = (ids[mapping.source], ids[mapping.target]);
                let key = format!(
                    "{}\u{1f}{source}\u{1f}{target}\u{1f}{}",
                    mapping.relationship_type,
                    properties.to_json()
                );
                if self.seen_relationships.insert(key) {
                    relationships.push(Relationship::new(
                        RelationshipId::new(),
                        self.run.graph_id,
                        mapping.relationship_type.clone(),
                        source,
                        target,
                        properties.clone(),
                    ));
                }
            }
        }
        self.summary.relationships_created += relationships.len() as u64;
        self.graph.create_entities(entities).await?;
        self.graph.create_relationships(relationships).await?;
        self.summary.records_processed += batch.len() as u64;
        #[allow(clippy::cast_precision_loss)] // A progress ratio; precision is irrelevant.
        let done = self.summary.records_processed as f64 / total.max(1) as f64;
        progress
            .report((1.0 - PREFLIGHT_SHARE).mul_add(done.min(1.0), PREFLIGHT_SHARE))
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use mindgraph_domain::{Properties, PropertyKey, PropertyValue};
    use mindgraph_graph::{
        Direction, GraphPath, GraphStoreError, MemoryGraphStore, NeighborQuery, PathQuery, Subgraph,
    };

    use super::*;
    use crate::mapping::tests::{account_mapping, customer_mapping, field, fraud_ontology, owns};
    use crate::{EntityMapping, IngestionMapping, MemorySourceStore, validate_mapping};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<f64>>);

    #[async_trait]
    impl ProgressSink for Recorder {
        async fn report(&self, progress: f64) {
            self.0.lock().unwrap().push(progress);
        }
    }

    struct Setup {
        graph: MemoryGraphStore,
        sources: MemorySourceStore,
        graph_id: GraphId,
    }

    fn setup() -> Setup {
        Setup {
            graph: MemoryGraphStore::new(),
            sources: MemorySourceStore::new(),
            graph_id: GraphId::new(),
        }
    }

    async fn ingest(
        s: &Setup,
        kind: DataSourceKind,
        data: &str,
        mapping: IngestionMapping,
        batch_size: usize,
    ) -> Result<(IngestionSummary, Vec<f64>), IngestionError> {
        let id = DataSourceId::new();
        s.sources.insert(id, data);
        let fields = data_fields(kind, data);
        let validated = validate_mapping(&mapping, &fraud_ontology(s.graph_id), &fields)?;
        let run = IngestionRun {
            graph_id: s.graph_id,
            data_source_id: id,
            kind,
            mapping: Arc::new(validated),
            limits: Limits::default(),
            batch_size,
        };
        let recorder = Recorder::default();
        let summary = run_ingestion(&run, &s.graph, &s.sources, &recorder).await?;
        let reports = recorder.0.lock().unwrap().clone();
        Ok((summary, reports))
    }

    fn data_fields(kind: DataSourceKind, data: &str) -> Vec<String> {
        crate::profile_dataset(kind, data.as_bytes(), &Limits::default(), &mut |_| {})
            .unwrap()
            .field_names()
    }

    async fn of_type(s: &Setup, entity_type: &str) -> Vec<Entity> {
        let search = EntitySearch::new()
            .with_entity_type(TypeName::new(entity_type).unwrap())
            .with_limit(1_000)
            .unwrap();
        s.graph.search_entities(s.graph_id, &search).await.unwrap()
    }

    fn customers_only() -> IngestionMapping {
        IngestionMapping {
            entities: vec![customer_mapping()],
            relationships: vec![],
        }
    }

    fn customers_and_accounts() -> IngestionMapping {
        IngestionMapping {
            entities: vec![customer_mapping(), account_mapping()],
            relationships: vec![owns("customer", "account")],
        }
    }

    #[tokio::test]
    async fn csv_creates_entities() {
        let s = setup();
        let (summary, reports) = ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\nC2,\n",
            customers_only(),
            500,
        )
        .await
        .unwrap();
        assert_eq!(
            summary,
            IngestionSummary {
                records_processed: 2,
                entities_created: 2,
                entities_matched: 0,
                relationships_created: 0
            }
        );
        let customers = of_type(&s, "Customer").await;
        assert_eq!(customers.len(), 2);
        assert!(
            !customers[1].properties().contains_key("name"),
            "blank optional values are omitted"
        );
        assert_eq!(reports.last().copied(), Some(1.0));
        assert!(
            reports.windows(2).all(|w| w[0] <= w[1]),
            "progress is monotonic"
        );
    }

    #[tokio::test]
    async fn json_creates_entities() {
        let s = setup();
        let data = r#"[{"customer_id":"C1","name":"Ann"},{"customer_id":"C2","name":null}]"#;
        let (summary, _) = ingest(&s, DataSourceKind::Json, data, customers_only(), 500)
            .await
            .unwrap();
        assert_eq!(summary.entities_created, 2);
    }

    #[tokio::test]
    async fn multi_entity_rows_dedupe_entities_and_relationships() {
        let s = setup();
        // C1 owns A1 twice (identical rows) and A2; three batches of size 1.
        let data =
            "customer_id,name,account_id,balance\nC1,Ann,A1,10.5\nC1,Ann,A1,10.5\nC1,Ann,A2,3\n";
        let (summary, _) = ingest(&s, DataSourceKind::Csv, data, customers_and_accounts(), 1)
            .await
            .unwrap();
        assert_eq!(summary.records_processed, 3);
        assert_eq!(summary.entities_created, 3, "one customer, two accounts");
        assert_eq!(
            summary.relationships_created, 2,
            "identical OWNS rows are de-duplicated"
        );
        let customer = &of_type(&s, "Customer").await[0];
        let neighbors = s
            .graph
            .get_neighbors(
                s.graph_id,
                customer.id(),
                &NeighborQuery::new(Direction::Outgoing),
            )
            .await
            .unwrap();
        assert_eq!(neighbors.relationships().len(), 2);
    }

    #[tokio::test]
    async fn create_or_match_reuses_existing_entities() {
        let s = setup();
        ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\n",
            customers_only(),
            500,
        )
        .await
        .unwrap();
        let (summary, _) = ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\nC2,Bob\n",
            customers_only(),
            500,
        )
        .await
        .unwrap();
        assert_eq!((summary.entities_created, summary.entities_matched), (1, 1));
        assert_eq!(of_type(&s, "Customer").await.len(), 2);
    }

    fn match_customer_then_account() -> IngestionMapping {
        IngestionMapping {
            entities: vec![
                EntityMapping {
                    mode: EntityMode::MatchOnly,
                    properties: vec![],
                    ..customer_mapping()
                },
                account_mapping(),
            ],
            relationships: vec![owns("customer", "account")],
        }
    }

    #[tokio::test]
    async fn match_only_links_to_existing_entities() {
        let s = setup();
        ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\n",
            customers_only(),
            500,
        )
        .await
        .unwrap();
        let data = "account_id,customer_id,balance\nA1,C1,5\nA2,C1,6\n";
        let (summary, _) = ingest(
            &s,
            DataSourceKind::Csv,
            data,
            match_customer_then_account(),
            500,
        )
        .await
        .unwrap();
        assert_eq!(
            summary.entities_matched, 1,
            "the cached customer is looked up once"
        );
        assert_eq!(
            (summary.entities_created, summary.relationships_created),
            (2, 2)
        );
    }

    #[tokio::test]
    async fn match_only_fails_for_missing_entities() {
        let s = setup();
        let err = ingest(
            &s,
            DataSourceKind::Csv,
            "account_id,customer_id,balance\nA1,C9,5\n",
            match_customer_then_account(),
            500,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, IngestionError::EntityNotFound { record: 1, .. }),
            "{err}"
        );
        assert!(
            of_type(&s, "Account").await.is_empty(),
            "the batch was not written"
        );
    }

    #[tokio::test]
    async fn ambiguous_identities_fail() {
        let s = setup();
        for _ in 0..2 {
            let mut properties = Properties::new();
            properties
                .insert(
                    PropertyKey::new("customer_id").unwrap(),
                    PropertyValue::from("C1"),
                )
                .unwrap();
            s.graph
                .create_entity(Entity::new(
                    EntityId::new(),
                    s.graph_id,
                    TypeName::new("Customer").unwrap(),
                    properties,
                ))
                .await
                .unwrap();
        }
        let err = ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\n",
            customers_only(),
            500,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, IngestionError::AmbiguousEntity { .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn bad_data_fails_before_any_write() {
        let s = setup();
        let mut mapping = customers_only();
        mapping.entities[0].properties.push(field("age", "age"));
        let data = "customer_id,name,age\nC1,Ann,30\nC2,Bob,thirty\n";
        let err = ingest(&s, DataSourceKind::Csv, data, mapping, 1)
            .await
            .unwrap_err();
        assert!(
            matches!(err, IngestionError::TypeConversion { record: 2, .. }),
            "{err}"
        );
        assert!(
            of_type(&s, "Customer").await.is_empty(),
            "preflight stopped the job"
        );

        let err = ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\n,Bob\n",
            customers_only(),
            1,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, IngestionError::MissingField { record: 2, .. }),
            "{err}"
        );
        assert!(of_type(&s, "Customer").await.is_empty());
    }

    #[tokio::test]
    async fn other_graphs_are_untouched() {
        let s = setup();
        let other = GraphId::new();
        ingest(
            &s,
            DataSourceKind::Csv,
            "customer_id,name\nC1,Ann\n",
            customers_only(),
            500,
        )
        .await
        .unwrap();
        let search = EntitySearch::new();
        assert!(
            s.graph
                .search_entities(other, &search)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Counts `search_entities` calls on top of the memory store.
    #[derive(Default)]
    struct Counting {
        inner: MemoryGraphStore,
        searches: AtomicUsize,
    }

    #[async_trait]
    impl GraphStore for Counting {
        async fn create_entity(&self, entity: Entity) -> Result<(), GraphStoreError> {
            self.inner.create_entity(entity).await
        }
        async fn create_entities(&self, entities: Vec<Entity>) -> Result<(), GraphStoreError> {
            self.inner.create_entities(entities).await
        }
        async fn create_relationship(&self, r: Relationship) -> Result<(), GraphStoreError> {
            self.inner.create_relationship(r).await
        }
        async fn create_relationships(&self, r: Vec<Relationship>) -> Result<(), GraphStoreError> {
            self.inner.create_relationships(r).await
        }
        async fn get_entity(
            &self,
            g: GraphId,
            e: EntityId,
        ) -> Result<Option<Entity>, GraphStoreError> {
            self.inner.get_entity(g, e).await
        }
        async fn search_entities(
            &self,
            g: GraphId,
            s: &EntitySearch,
        ) -> Result<Vec<Entity>, GraphStoreError> {
            self.searches.fetch_add(1, Ordering::Relaxed);
            self.inner.search_entities(g, s).await
        }
        async fn get_neighbors(
            &self,
            g: GraphId,
            e: EntityId,
            q: &NeighborQuery,
        ) -> Result<Subgraph, GraphStoreError> {
            self.inner.get_neighbors(g, e, q).await
        }
        async fn find_path(
            &self,
            g: GraphId,
            a: EntityId,
            b: EntityId,
            q: &PathQuery,
        ) -> Result<Option<GraphPath>, GraphStoreError> {
            self.inner.find_path(g, a, b, q).await
        }
        async fn get_subgraph(
            &self,
            g: GraphId,
            ids: &[EntityId],
        ) -> Result<Subgraph, GraphStoreError> {
            self.inner.get_subgraph(g, ids).await
        }
        async fn delete_entity(&self, g: GraphId, e: EntityId) -> Result<bool, GraphStoreError> {
            self.inner.delete_entity(g, e).await
        }
        async fn clear_graph(&self, g: GraphId) -> Result<(), GraphStoreError> {
            self.inner.clear_graph(g).await
        }
    }

    async fn ingest_into(
        graph: &dyn GraphStore,
        graph_id: GraphId,
        data: &str,
        mapping: &IngestionMapping,
        batch_size: usize,
    ) -> Result<IngestionSummary, IngestionError> {
        let sources = MemorySourceStore::new();
        let id = DataSourceId::new();
        sources.insert(id, data);
        let fields = data_fields(DataSourceKind::Csv, data);
        let run = IngestionRun {
            graph_id,
            data_source_id: id,
            kind: DataSourceKind::Csv,
            mapping: Arc::new(validate_mapping(
                mapping,
                &fraud_ontology(graph_id),
                &fields,
            )?),
            limits: Limits::default(),
            batch_size,
        };
        run_ingestion(&run, graph, &sources, &Recorder::default()).await
    }

    fn customer(graph_id: GraphId, customer_id: &str) -> Entity {
        let mut properties = Properties::new();
        properties
            .insert(
                PropertyKey::new("customer_id").unwrap(),
                PropertyValue::from(customer_id),
            )
            .unwrap();
        Entity::new(
            EntityId::new(),
            graph_id,
            TypeName::new("Customer").unwrap(),
            properties,
        )
    }

    #[tokio::test]
    async fn identity_lookups_are_batched_not_per_row() {
        let store = Counting::default();
        let graph_id = GraphId::new();
        let customers: Vec<Entity> = (0..600)
            .map(|i| customer(graph_id, &format!("C{i:04}")))
            .collect();
        store.inner.create_entities(customers).await.unwrap();
        let mut data = String::from("account_id,customer_id,balance\n");
        for i in 0..600 {
            data.push_str(&format!("A{i:04},C{i:04},1\n"));
        }
        let summary = ingest_into(&store, graph_id, &data, &match_customer_then_account(), 500)
            .await
            .unwrap();
        assert_eq!(
            (summary.entities_matched, summary.relationships_created),
            (600, 600)
        );
        // 1 "is Account empty" probe + one lookup per batch (500 + 100
        // identities). Not 600.
        assert_eq!(store.searches.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn truncated_lookups_are_confirmed_one_by_one() {
        let store = MemoryGraphStore::new();
        let graph_id = GraphId::new();
        // 1,001 entities share one identity and sort before C1, so a combined
        // search is truncated before it reaches C1.
        let duplicates: Vec<Entity> = (0..1_001).map(|_| customer(graph_id, "DUP")).collect();
        store.create_entities(duplicates).await.unwrap();
        store.create_entity(customer(graph_id, "C1")).await.unwrap();
        let data = "account_id,customer_id,balance\nA1,C1,1\nA2,DUP,2\n";
        let err = ingest_into(&store, graph_id, data, &match_customer_then_account(), 500)
            .await
            .unwrap_err();
        assert!(
            matches!(err, IngestionError::AmbiguousEntity { record: 2, .. }),
            "record 1 (C1) must resolve; record 2 is ambiguous: {err}"
        );
    }
}
