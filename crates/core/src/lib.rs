//! skjalf-core — pure, display-free logic for the Steam Deck IPTV app.
//!
//! Everything in this crate is deterministic and I/O-free except through
//! explicitly injected traits (`HttpFetcher`) or file paths (`store`), so the
//! whole app behavior is unit-testable without a display, network, or libmpv.
pub mod art_cache;
pub mod browse;
pub mod catalog;
pub mod display;
pub mod m3u;
pub mod model;
pub mod nav;
pub mod progress;
pub mod store;
pub mod template;
pub mod tracks;
pub mod xtream;
