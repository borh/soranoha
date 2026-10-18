//! Coverage matrix loader and schema validator for
//! `data/aozora-syntax-coverage.toml`, whose JSON Schema is
//! `schemas/records/aozora-syntax-coverage.schema.json`.

pub mod corpus_check;
pub mod detectors;
pub mod edition_diff;
pub mod fidelity_report;
pub mod matrix;
pub mod merge;
pub mod occurrence_alignment;
pub mod parser_exec;
pub mod residue;
pub mod schema;
pub mod source_accountability;
pub mod source_corpus;
pub mod source_inventory;

pub use matrix::{
    AdapterCell, CorpusPrevalence, CoverageBasis, CoverageMatrix, ParserCell, ParserComparison,
    ParserComparisonRow, Recognition, Row, RowAatFidelity,
};
pub use schema::{RowError, SchemaValidator, ValidationOptions};
