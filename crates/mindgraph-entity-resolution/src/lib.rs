//! Basic entity resolution for MindGraph Open (ADR-009).
//!
//! v0.1 detects candidate duplicates using exact source identifiers,
//! normalised emails, phones and strings, and fuzzy name similarity.
//! Uncertain fuzzy matches are never merged automatically, and every merge
//! preserves its provenance.
//!
//! Status: skeleton. Implemented in M9.
