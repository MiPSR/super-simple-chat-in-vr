//! Independently versioned creator Luau API. Additive capabilities keep the
//! version; breaking signature or semantic changes bump it. The manifest
//! schema and wire protocol have their own version numbers.

/// Current creator API level advertised to scripts as `world.api_version`
/// (`avatar.api_version` on client avatar scripts).
pub const VERSION: u32 = 1;
