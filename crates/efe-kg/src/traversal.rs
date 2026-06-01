// SPDX-License-Identifier: UNLICENSED
//! M33 — Graph traversal algorithms.
//!
//! Three algorithms ship in M33 :
//!
//! - **BFS** : k-hop expansion from a seed.
//! - **DFS** : depth-first variant (rare for our use cases).
//! - **Shortest path** : single-source single-target (BFS edge-
//!   weight = 1).
//!
//! All capped by `MAX_HOPS` + `MAX_FANOUT_PER_HOP` (ADR-M33-006).

use std::collections::{HashSet, VecDeque};

use crate::error::{KgError, Result};
use crate::graph::Graph;
use crate::relation::Predicate;

/// Hard cap on hop count (cf. ADR-M33-006).
pub const MAX_HOPS: usize = 6;

/// Hard cap on per-hop fan-out.
pub const MAX_FANOUT_PER_HOP: usize = 256;

/// One traversal result row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraversalNode {
    /// Entity id.
    pub entity_id: String,
    /// Hop distance from the seed (0 = seed itself).
    pub hop_distance: usize,
    /// Path of relation ids leading from the seed.
    pub path: Vec<String>,
}

/// Filter predicates allowed at traversal time.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TraversalFilter {
    /// Only follow edges whose predicate is in this set. Empty ⇒
    /// all predicates allowed.
    #[serde(default)]
    pub allowed_predicates: Vec<Predicate>,
    /// Skip these entity ids (e.g. already-seen seeds).
    #[serde(default)]
    pub skip_entity_ids: Vec<String>,
}

impl TraversalFilter {
    /// `true` iff the filter accepts the predicate.
    #[must_use]
    pub fn accepts_predicate(&self, pred: &Predicate) -> bool {
        self.allowed_predicates.is_empty() || self.allowed_predicates.contains(pred)
    }

    /// `true` iff the entity is on the skip-list.
    #[must_use]
    pub fn is_skipped(&self, id: &str) -> bool {
        self.skip_entity_ids.iter().any(|s| s == id)
    }
}

/// Visited-set entry tracking BFS parent pointers (M33 perf G2).
/// `parent_id = None` for the seed.
#[derive(Debug, Clone)]
struct BfsRecord {
    hop: usize,
    parent_id: Option<String>,
    via_rel: Option<String>,
}

/// BFS k-hop expansion from a seed entity.
///
/// Returns at most `max_results` nodes ordered by `(hop_distance,
/// entity_id)`. The seed is included (hop 0).
///
/// **M33 perf (G1+G2+G3)** rewrite :
///
/// - `Graph::for_each_out_edge` callback removes the per-node
///   Vec clone (G1).
/// - Parent-pointer storage means we no longer clone `path` on
///   every queue push ; instead we materialize paths once at the
///   end, only for the `max_results` survivors (G2).
/// - `visited` HashMap + `queue` VecDeque are pre-sized with a
///   coarse upper bound (G3) to avoid the realloc-growth cascade.
pub fn bfs(
    graph: &Graph,
    seed_id: &str,
    max_hops: usize,
    filter: &TraversalFilter,
    max_results: usize,
) -> Result<Vec<TraversalNode>> {
    if max_hops > MAX_HOPS {
        return Err(KgError::TraversalCapExceeded {
            observed: max_hops,
            cap: MAX_HOPS,
        });
    }
    if max_results == 0 {
        return Ok(Vec::new());
    }
    // Verify the seed exists.
    let _ = graph.get_entity(seed_id)?;

    // G3 : pre-size — at most max_results × 4 fan-out / hop. Cheap
    // upper bound that avoids rehash cascades.
    let est = max_results.saturating_mul(4).max(64);
    let mut visited: std::collections::HashMap<String, BfsRecord> =
        std::collections::HashMap::with_capacity(est);
    let mut queue: VecDeque<String> = VecDeque::with_capacity(est);
    let mut out_ids: Vec<String> = Vec::with_capacity(max_results);

    visited.insert(
        seed_id.to_string(),
        BfsRecord {
            hop: 0,
            parent_id: None,
            via_rel: None,
        },
    );
    queue.push_back(seed_id.to_string());

    'outer: while let Some(cur_id) = queue.pop_front() {
        // We need (cur_hop) → look up via the visited map.
        let cur_hop = visited
            .get(&cur_id)
            .map(|r| r.hop)
            .expect("queue entries must exist in visited");
        if !filter.is_skipped(&cur_id) {
            out_ids.push(cur_id.clone());
            if out_ids.len() >= max_results {
                break;
            }
        }
        if cur_hop >= max_hops {
            continue;
        }
        // G1 : iterate the adjacency list without cloning.
        let mut fan_count = 0_usize;
        let mut neighbours: Vec<(String, String)> = Vec::new();
        graph.for_each_out_edge(&cur_id, |(pred, nbr, rel_id)| {
            if fan_count >= MAX_FANOUT_PER_HOP {
                return;
            }
            if !filter.accepts_predicate(pred) {
                return;
            }
            if visited.contains_key(nbr) {
                return;
            }
            // Buffer the additions ; we'll insert outside the
            // read-lock scope (the closure is invoked while the
            // graph read lock is held).
            neighbours.push((nbr.clone(), rel_id.clone()));
            fan_count += 1;
        });

        for (nbr, rel_id) in neighbours {
            visited.insert(
                nbr.clone(),
                BfsRecord {
                    hop: cur_hop + 1,
                    parent_id: Some(cur_id.clone()),
                    via_rel: Some(rel_id),
                },
            );
            queue.push_back(nbr);
            // Sanity safeguard against pathological adversarial
            // graphs : if `visited` grows past `est * 4` (= ~
            // 16 × max_results) we abort early. The pre-existing
            // `MAX_HOPS` + `MAX_FANOUT_PER_HOP` caps already bound
            // the worst case but this provides defence-in-depth.
            // Formula simplified : `out_ids ⊆ visited`, so the
            // earlier `out_ids + (visited − out_ids)` was just
            // `visited.len()`.
            if visited.len() > est.saturating_mul(4) {
                break 'outer;
            }
        }
    }

    // G2 : materialize paths once at the end, only for the
    // surviving out_ids.
    let mut out: Vec<TraversalNode> = Vec::with_capacity(out_ids.len());
    for id in out_ids {
        let rec = visited
            .get(&id)
            .expect("out_ids subset of visited");
        let path = materialize_path(&visited, &id);
        out.push(TraversalNode {
            entity_id: id,
            hop_distance: rec.hop,
            path,
        });
    }
    Ok(out)
}

/// Walk parent pointers from `id` back to the seed, returning the
/// `via_rel` ids in seed→id order.
fn materialize_path(
    visited: &std::collections::HashMap<String, BfsRecord>,
    id: &str,
) -> Vec<String> {
    let mut rev: Vec<String> = Vec::new();
    let mut cur = id;
    while let Some(rec) = visited.get(cur) {
        if let Some(rel) = &rec.via_rel {
            rev.push(rel.clone());
        }
        match &rec.parent_id {
            Some(p) => cur = p.as_str(),
            None => break,
        }
    }
    rev.reverse();
    rev
}

/// DFS k-hop expansion. Same caps as BFS. Returned in DFS order.
pub fn dfs(
    graph: &Graph,
    seed_id: &str,
    max_hops: usize,
    filter: &TraversalFilter,
    max_results: usize,
) -> Result<Vec<TraversalNode>> {
    if max_hops > MAX_HOPS {
        return Err(KgError::TraversalCapExceeded {
            observed: max_hops,
            cap: MAX_HOPS,
        });
    }
    if max_results == 0 {
        return Ok(Vec::new());
    }
    let _ = graph.get_entity(seed_id)?;
    let mut visited: HashSet<String> = HashSet::new();
    let mut out: Vec<TraversalNode> = Vec::new();
    dfs_recurse(
        graph,
        seed_id,
        max_hops,
        filter,
        max_results,
        &mut visited,
        &mut out,
        TraversalNode {
            entity_id: seed_id.to_string(),
            hop_distance: 0,
            path: Vec::new(),
        },
    );
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn dfs_recurse(
    graph: &Graph,
    _seed_id: &str,
    max_hops: usize,
    filter: &TraversalFilter,
    max_results: usize,
    visited: &mut HashSet<String>,
    out: &mut Vec<TraversalNode>,
    cur: TraversalNode,
) {
    if out.len() >= max_results {
        return;
    }
    if !visited.insert(cur.entity_id.clone()) {
        return;
    }
    if !filter.is_skipped(&cur.entity_id) {
        out.push(cur.clone());
    }
    if cur.hop_distance >= max_hops {
        return;
    }
    let outgoing = graph.out_edges(&cur.entity_id);
    let mut fan_count = 0_usize;
    for (pred, nbr, rel_id) in outgoing {
        if fan_count >= MAX_FANOUT_PER_HOP {
            break;
        }
        if !filter.accepts_predicate(&pred) {
            continue;
        }
        if visited.contains(&nbr) {
            continue;
        }
        let mut new_path = cur.path.clone();
        new_path.push(rel_id);
        dfs_recurse(
            graph,
            _seed_id,
            max_hops,
            filter,
            max_results,
            visited,
            out,
            TraversalNode {
                entity_id: nbr,
                hop_distance: cur.hop_distance + 1,
                path: new_path,
            },
        );
        fan_count += 1;
    }
}

/// Shortest path (unit edge weight) from `source` to `target`.
/// Returns the path as a `Vec<TraversalNode>` (inclusive of both
/// endpoints) or `None` if unreachable within `max_hops`.
pub fn shortest_path(
    graph: &Graph,
    source: &str,
    target: &str,
    max_hops: usize,
    filter: &TraversalFilter,
) -> Result<Option<Vec<TraversalNode>>> {
    if max_hops > MAX_HOPS {
        return Err(KgError::TraversalCapExceeded {
            observed: max_hops,
            cap: MAX_HOPS,
        });
    }
    if source == target {
        let _ = graph.get_entity(source)?;
        return Ok(Some(vec![TraversalNode {
            entity_id: source.to_string(),
            hop_distance: 0,
            path: Vec::new(),
        }]));
    }
    let _ = graph.get_entity(source)?;
    let _ = graph.get_entity(target)?;
    // BFS storing (parent_entity_id, relation_id) for backtrack.
    let mut visited: HashSet<String> = HashSet::new();
    let mut parents: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    visited.insert(source.to_string());
    queue.push_back((source.to_string(), 0));

    while let Some((cur, dist)) = queue.pop_front() {
        if dist >= max_hops {
            continue;
        }
        let outgoing = graph.out_edges(&cur);
        let mut fan_count = 0_usize;
        for (pred, nbr, rel_id) in outgoing {
            if fan_count >= MAX_FANOUT_PER_HOP {
                break;
            }
            if !filter.accepts_predicate(&pred) {
                continue;
            }
            if visited.contains(&nbr) {
                continue;
            }
            visited.insert(nbr.clone());
            parents.insert(nbr.clone(), (cur.clone(), rel_id.clone()));
            if nbr == target {
                // Backtrack.
                let mut rev_path: Vec<TraversalNode> = Vec::new();
                let mut node = target.to_string();
                let mut hop = dist + 1;
                loop {
                    let path_rels: Vec<String> = parents
                        .iter()
                        .filter(|(k, _)| **k == node)
                        .map(|(_, (_, r))| r.clone())
                        .collect();
                    rev_path.push(TraversalNode {
                        entity_id: node.clone(),
                        hop_distance: hop,
                        path: path_rels,
                    });
                    if node == source {
                        break;
                    }
                    if let Some((p, _)) = parents.get(&node) {
                        node = p.clone();
                        hop -= 1;
                    } else {
                        break;
                    }
                }
                rev_path.reverse();
                return Ok(Some(rev_path));
            }
            queue.push_back((nbr, dist + 1));
            fan_count += 1;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{Entity, EntityKind};
    use crate::relation::Relation;

    fn graph_chain(n: usize) -> Graph {
        let g = Graph::new();
        for i in 0..n {
            g.upsert_entity(Entity::new(format!("e{i}"), EntityKind::Misc, format!("e{i}")))
                .unwrap();
        }
        for i in 0..n - 1 {
            g.upsert_relation(Relation::new(
                format!("r{i}"),
                format!("e{i}"),
                Predicate::new("relates_to").unwrap(),
                format!("e{}", i + 1),
                0.9,
            ))
            .unwrap();
        }
        g
    }

    #[test]
    fn bfs_explores_in_hop_order() {
        let g = graph_chain(5);
        let out = bfs(&g, "e0", 3, &TraversalFilter::default(), 100).unwrap();
        // BFS from e0 hops 0..3 ⇒ visit e0, e1, e2, e3 (in that order).
        let ids: Vec<&str> = out.iter().map(|n| n.entity_id.as_str()).collect();
        assert_eq!(ids, vec!["e0", "e1", "e2", "e3"]);
        let dists: Vec<usize> = out.iter().map(|n| n.hop_distance).collect();
        assert_eq!(dists, vec![0, 1, 2, 3]);
    }

    #[test]
    fn bfs_respects_max_hops() {
        let g = graph_chain(5);
        let out = bfs(&g, "e0", 1, &TraversalFilter::default(), 100).unwrap();
        assert_eq!(out.len(), 2); // hop 0 + hop 1
    }

    #[test]
    fn bfs_respects_max_results() {
        let g = graph_chain(10);
        let out = bfs(&g, "e0", 6, &TraversalFilter::default(), 3).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn bfs_filter_skips_unsupported_predicates() {
        let g = graph_chain(3);
        let filter = TraversalFilter {
            allowed_predicates: vec![Predicate::new("works_at").unwrap()],
            skip_entity_ids: vec![],
        };
        let out = bfs(&g, "e0", 5, &filter, 100).unwrap();
        // Only the seed survives — `relates_to` edges are filtered out.
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn bfs_rejects_max_hops_above_cap() {
        let g = graph_chain(3);
        let e = bfs(&g, "e0", MAX_HOPS + 1, &TraversalFilter::default(), 100).unwrap_err();
        assert!(matches!(e, KgError::TraversalCapExceeded { .. }));
    }

    #[test]
    fn bfs_seed_must_exist() {
        let g = Graph::new();
        let e = bfs(&g, "missing", 2, &TraversalFilter::default(), 100).unwrap_err();
        assert!(matches!(e, KgError::EntityNotFound(_)));
    }

    #[test]
    fn dfs_visits_all_reachable_in_chain() {
        let g = graph_chain(5);
        let out = dfs(&g, "e0", 5, &TraversalFilter::default(), 100).unwrap();
        let ids: Vec<&str> = out.iter().map(|n| n.entity_id.as_str()).collect();
        // DFS in a chain returns e0..e4.
        assert_eq!(ids.len(), 5);
        assert_eq!(ids[0], "e0");
    }

    #[test]
    fn shortest_path_finds_direct_neighbour() {
        let g = graph_chain(3);
        let p = shortest_path(&g, "e0", "e1", 5, &TraversalFilter::default()).unwrap();
        let p = p.unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].entity_id, "e0");
        assert_eq!(p[1].entity_id, "e1");
    }

    #[test]
    fn shortest_path_finds_multi_hop() {
        let g = graph_chain(5);
        let p = shortest_path(&g, "e0", "e3", 5, &TraversalFilter::default()).unwrap();
        let p = p.unwrap();
        assert_eq!(p.first().unwrap().entity_id, "e0");
        assert_eq!(p.last().unwrap().entity_id, "e3");
    }

    #[test]
    fn shortest_path_self_returns_self() {
        let g = graph_chain(3);
        let p = shortest_path(&g, "e0", "e0", 5, &TraversalFilter::default()).unwrap();
        let p = p.unwrap();
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn shortest_path_unreachable_returns_none() {
        let g = Graph::new();
        g.upsert_entity(Entity::new("e1", EntityKind::Misc, "a")).unwrap();
        g.upsert_entity(Entity::new("e2", EntityKind::Misc, "b")).unwrap();
        let p = shortest_path(&g, "e1", "e2", 3, &TraversalFilter::default()).unwrap();
        assert!(p.is_none());
    }
}
