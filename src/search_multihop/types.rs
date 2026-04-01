use crate::memory::Memory;

pub struct MultiHopResult {
    pub memories: Vec<Memory>,
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}
