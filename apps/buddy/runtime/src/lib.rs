#[cfg(feature = "runtime")]
mod agents;
#[cfg(feature = "runtime")]
mod app_paths;
#[cfg(any(feature = "runtime", feature = "pet"))]
mod choreography;
#[cfg(feature = "runtime")]
mod commands;
#[cfg(feature = "runtime")]
mod context_pack;
#[cfg(feature = "runtime")]
mod domain;
#[cfg(any(feature = "runtime", feature = "pet"))]
mod error;
#[cfg(feature = "runtime")]
mod health_check;
#[cfg(feature = "runtime")]
mod intent;
#[cfg(feature = "pet")]
mod kwin_scripting;
#[cfg(feature = "runtime")]
pub mod local_log;
#[cfg(feature = "runtime")]
mod memory;
#[cfg(any(feature = "runtime", feature = "pet"))]
mod native_pet;
#[cfg(feature = "runtime")]
pub mod protocol;
#[cfg(feature = "runtime")]
pub mod runtime;
#[cfg(feature = "runtime")]
mod runtime_instance;
#[cfg(feature = "runtime")]
pub mod server;
#[cfg(feature = "runtime")]
mod state;
#[cfg(feature = "runtime")]
mod storage;

#[cfg(feature = "runtime")]
pub use choreography::run_affective_state_command_from_env;
#[cfg(feature = "runtime")]
pub use health_check::run_headless_command_from_env;
#[cfg(feature = "pet")]
pub use native_pet::{
    run_native_pet_drag_replay_command_from_env, run_native_pet_sidecar_from_env,
    run_native_pet_smoke_command_from_env,
};
