mod error;
mod migration;
mod model;
mod operation;
pub mod snapshot;
mod store;

pub use error::{StoreError, StoreResult};
pub use migration::{CURRENT_SCHEMA_VERSION, MIGRATION_10, MIGRATION_11};
pub use model::*;
pub use snapshot::{
    EncodedSnapshot, ExportProjectSnapshot, ExportSnapshotBlock, ProjectSnapshotV1, SNAPSHOT_CODEC,
    SNAPSHOT_CODEC_VERSION, SNAPSHOT_SCHEMA_VERSION, SnapshotBlock, SnapshotBranch,
    SnapshotCodecError, SnapshotCommit, SnapshotDocument, SnapshotProject, checksum,
    checksum as snapshot_checksum, compare_documents, decode_snapshot, encode_snapshot,
    lcs_by_order_key,
};
pub use store::{MINIMUM_SQLITE_VERSION, OptimizerStore, StoreDiagnostics};
