mod error;
mod migration;
mod model;
mod operation;
mod snapshot;
mod store;

pub use error::{StoreError, StoreResult};
pub use migration::CURRENT_SCHEMA_VERSION;
pub use model::*;
pub use snapshot::{
    EncodedSnapshot, ProjectSnapshotV1, SNAPSHOT_CODEC, SNAPSHOT_CODEC_VERSION,
    SNAPSHOT_SCHEMA_VERSION, SnapshotBlock, SnapshotBranch, SnapshotCodecError, SnapshotCommit,
    SnapshotDocument, SnapshotProject, checksum as snapshot_checksum, decode_snapshot,
    encode_snapshot,
};
pub use store::{MINIMUM_SQLITE_VERSION, OptimizerStore, StoreDiagnostics};
