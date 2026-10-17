mod chain;
mod decision;
mod fences;
mod front_pass;
#[cfg(test)]
use super::Pipeline;
#[cfg(test)]
use decision::{ResidentDecision, ResidentFallbackReason};
#[cfg(test)]
mod tests;
