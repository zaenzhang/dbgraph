//! Database provider abstractions and concrete database integrations.

pub mod postgres;

pub use postgres::{
    canonicalize_raw_snapshot, ConnectionInfo, DatabaseProvider, PostgresProvider,
    ProviderConnectionConfig, ProviderRegistry, RawColumn, RawColumnStatistics, RawConstraint,
    RawConstraintKind, RawEnum, RawIndex, RawRoutine, RawRoutineKind, RawSchema, RawSchemaSnapshot,
    RawSequence, RawStatisticsSnapshot, RawTable, RawTableStatistics, RawTrigger, RawView,
};
