pub mod cli;
pub mod doctor;
pub mod error;
pub mod model;
pub mod paths;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_COMMIT: &str = env!("SMP_BUILD_COMMIT");
pub const MACHINE_SCHEMA_VERSION: u32 = 1;
pub const REQUEST_SCHEMA_VERSION: u32 = 1;
pub const RESPONSE_SCHEMA_VERSION: u32 = 1;
