//! The Bodo Kit processing graph.
//!
//! - [`dag`]: [`IncrementalDag`], which keeps a topological order as edges are added and refuses edges that would
//!   close a cycle, naming the cycle.
//! - [`plan`]: describing a graph ([`GraphDesc`]) and compiling it into a [`Plan`]: Kahn ordering, plugin delay
//!   compensation, and every buffer and delay line allocated up front.
//! - [`exec`]: running plans block by block on an RT pool ([`GraphRunner`]), in parallel and deterministically.

#![forbid(unsafe_code)]

pub mod dag;
pub mod exec;
pub mod plan;

pub use dag::{DagError, DagNode, IncrementalDag};
pub use exec::{GraphRunner, NodeIo, PlanSender, Processor, runner};
pub use plan::{CompileError, Compiled, EdgeDesc, EdgeGains, EdgeKey, GraphDesc, LatencyReport, Layout, NodeDesc, NodeKey, Plan, compile};
