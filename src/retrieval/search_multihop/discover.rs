use std::collections::HashSet;

use crate::memory::Memory;

pub(crate) fn discover_entities(query: &str, first_hop: &[Memory]) -> Vec<String> {
    let mut discovered_entities = Vec::new();
    let mut seen_entities: HashSet<String> = HashSet::new();

    for entity in crate::entity::extract_entities(query, "") {
        seen_entities.insert(entity.to_lowercase());
    }

    for memory in first_hop {
        for entity in crate::entity::extract_entities(&memory.title, &memory.text) {
            let lower = entity.to_lowercase();
            if !seen_entities.contains(&lower) {
                seen_entities.insert(lower);
                discovered_entities.push(entity);
            }
        }
    }

    discovered_entities
}
