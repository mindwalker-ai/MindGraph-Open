//! Ontology management for MindGraph Open (ADR-005, ADR-008).
//!
//! The ontology model itself (`Ontology`, `EntityType`, `RelationshipType`,
//! `OntologyVersion`) lives in `mindgraph-domain`. This crate will hold the
//! workflows around it: drafting ontologies from AI suggestions, user review,
//! and activating a new ontology version. AI suggestions remain drafts until
//! the user confirms "Generate Graph".
//!
//! Status: skeleton. Implemented in M3 and M5.
