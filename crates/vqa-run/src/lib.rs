//! The session: settings, the capability cache, and one comparison.
//!
//! This crate holds no interface code and no `egui` dependency. That is what keeps a
//! command line cheap to add later.

pub mod cache;
pub mod csv_writer;
pub mod export;
pub mod graph_png;
pub mod record;
pub mod session;
pub mod settings;
pub mod supervisor;
pub mod vmaf_models;

pub use cache::CapabilityCache;
pub use csv_writer::{MetricColumn, SummaryRow, write_frame_csv, write_summary_csv};
pub use export::{Exported, write_run};
pub use graph_png::png_from_svg;
pub use record::{InvocationRecord, RunOutcome, RunRecord};
pub use session::{BinaryScan, Selection, Session, scan_binaries};
pub use settings::{BUTTERAUGLI_PRESET_NITS, Settings, ThemeChoice, VIEWING_DISTANCES};
pub use supervisor::{
    EncodeWork, RealProcessRunner, SupervisorEvent, run_plan, run_plan_with_cancel,
};
