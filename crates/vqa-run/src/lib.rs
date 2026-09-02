//! The session: settings, the capability cache, and one comparison.
//!
//! This crate holds no interface code and no `egui` dependency. That is what keeps a
//! command line cheap to add later.

pub mod cache;
pub mod csv_writer;
pub mod session;
pub mod settings;
pub mod supervisor;
pub mod vmaf_models;

pub use cache::CapabilityCache;
pub use csv_writer::{MetricColumn, write_frame_csv};
pub use session::{Selection, Session};
pub use settings::{BUTTERAUGLI_PRESET_NITS, Settings, ThemeChoice, VIEWING_DISTANCES};
pub use supervisor::{EncodeWork, RealProcessRunner, SupervisorEvent, run_plan};
