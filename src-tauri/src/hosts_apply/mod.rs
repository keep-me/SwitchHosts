//! Hosts apply pipeline.
//!
//! `aggregate` walks the renderer-shaped manifest tree, picks every
//! node whose `on === true`, concatenates the matching
//! `entries/<id>.hosts` files in tree order, and optionally runs the
//! duplicate-domain dedup pass that mirrors
//! `src/common/normalize.ts::removeDuplicateRecords`.
//!
//! P2.E.1 only exposes the aggregation step (preview-only). Privileged
//! writes to `/etc/hosts` land in P2.E.2 alongside the platform-specific
//! elevation helpers.

pub mod aggregate;

pub use aggregate::aggregate_selected_content;
