//! Stable cluster centres and a linear-size scene. Layout is rebuilt when records
//! change. Camera motion reuses coordinates, edge indices and neighbour lists.
use sovra_core::knowledge_map::{Layer, MapPage};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Scene {
    pub nodes: Vec<Dot>,
    pub edges: Vec<Edge>,
    pub adjacent: Vec<Vec<usize>>,
    pub by_id: BTreeMap<String, usize>,
    pub by_x: Vec<usize>,
}
pub(super) struct Dot {
    pub id: String,
    pub label: String,
    pub layer: Layer,
    pub hub: bool,
    pub x: f32,
    pub y: f32,
}
pub(super) struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub membership: bool,
}
impl Scene {
    #[expect(
        clippy::cast_precision_loss,
        reason = "Map-space coordinates are approximate; record counts and identities stay integral."
    )]
    pub(super) fn build(data: &MapPage, linked: bool) -> Self {
        let mut scene = Self::default();
        let mut counts = [0usize; 7];
        let mut seen = [0usize; 7];
        for n in data.nodes.values() {
            if n.id != n.layer.hub() {
                counts[n.layer as usize] += 1;
            }
        }
        for n in data.nodes.values() {
            let slot = n.layer as usize;
            // A generous central corridor leaves long cross-collection edges visible.
            let (cx, cy) = [
                (220., 265.),
                (490., 195.),
                (780., 270.),
                (775., 673.),
                (525., 785.),
                (245., 740.),
                (190., 505.),
            ][slot];
            let hub = n.id == n.layer.hub();
            let (x, y) = if hub {
                (cx, cy)
            } else {
                let series = seen[slot];
                seen[slot] += 1;
                let angle = series as f32 * 2.399_963_1 + slot as f32;
                let radius = ((series + 1) as f32 / counts[slot].max(1) as f32).sqrt();
                (
                    cx + angle.cos() * radius * 132.,
                    cy + angle.sin() * radius * 152.,
                )
            };
            scene.by_id.insert(n.id.clone(), scene.nodes.len());
            let mut label = n
                .label
                .trim()
                .chars()
                .map(|c| if c.is_whitespace() { ' ' } else { c });
            let mut chars: String = label.by_ref().take(35).collect();
            if label.next().is_some() {
                chars.push('…');
            }
            scene.nodes.push(Dot {
                id: n.id.clone(),
                label: chars,
                layer: n.layer,
                hub,
                x,
                y,
            });
        }
        scene.adjacent = vec![vec![]; scene.nodes.len()];
        for e in &data.links {
            if let (Some(&from), Some(&to)) = (scene.by_id.get(&e.from), scene.by_id.get(&e.to)) {
                let i = scene.edges.len();
                scene.edges.push(Edge {
                    from,
                    to,
                    label: e.label.clone(),
                    membership: e.membership,
                });
                scene.adjacent[from].push(i);
                scene.adjacent[to].push(i);
            }
        }
        if linked {
            scene.arrange_connections();
        }
        // The camera preserves horizontal order. Pointer moves can search a
        // narrow slice of the scene even after panning or zooming.
        scene.by_x = (0..scene.nodes.len()).collect();
        scene
            .by_x
            .sort_unstable_by(|&a, &b| scene.nodes[a].x.total_cmp(&scene.nodes[b].x));
        scene
    }

    // Twelve linear passes. Each small grid cell supplies one centre of mass
    // for repulsion, avoiding a pairwise force between every two records.
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "Clamped finite map-space positions select one of the 40 grid cells on each axis."
    )]
    fn arrange_connections(&mut self) {
        const SIDE: usize = 40;
        let cell = |v: f32| (v / 25.).clamp(0., 39.) as usize;
        let mut delta = vec![(0., 0.); self.nodes.len()];
        let mut grid = vec![(0_f32, 0_f32, 0_f32); SIDE * SIDE];
        for _ in 0..12 {
            grid.fill((0., 0., 0.));
            for node in &self.nodes {
                let bin = &mut grid[cell(node.y) * SIDE + cell(node.x)];
                bin.0 += node.x;
                bin.1 += node.y;
                bin.2 += 1.;
            }
            for (i, node) in self.nodes.iter().enumerate() {
                if node.hub {
                    continue;
                }
                let mut dx = 0.;
                let mut dy = 0.;
                let mut sum = 0_f32;
                for &j in &self.adjacent[i] {
                    let edge = &self.edges[j];
                    let other = &self.nodes[if edge.from == i { edge.to } else { edge.from }];
                    let weight = if edge.membership { 0.18 } else { 1. };
                    dx += (other.x - node.x) * weight;
                    dy += (other.y - node.y) * weight;
                    sum += weight;
                }
                dx = dx / sum.max(1_f32) * 0.10;
                dy = dy / sum.max(1_f32) * 0.10;
                let x = cell(node.x);
                let y = cell(node.y);
                for row in y.saturating_sub(1)..=(y + 1).min(SIDE - 1) {
                    for col in x.saturating_sub(1)..=(x + 1).min(SIDE - 1) {
                        let (sx, sy, n) = grid[row * SIDE + col];
                        if n == 0. {
                            continue;
                        }
                        let rx = node.x - sx / n;
                        let ry = node.y - sy / n;
                        let force = 48. * n.min(12.) / (rx * rx + ry * ry + 16.);
                        dx += rx * force;
                        dy += ry * force;
                    }
                }
                delta[i] = (dx.clamp(-14., 14.), dy.clamp(-14., 14.));
            }
            for (node, (dx, dy)) in self.nodes.iter_mut().zip(&delta) {
                node.x = (node.x + dx).clamp(12., 988.);
                node.y = (node.y + dy).clamp(12., 988.);
            }
        }
    }
    pub(super) fn neighbours(
        &self,
        node: usize,
        membership: bool,
        relationships: bool,
    ) -> impl Iterator<Item = usize> {
        self.adjacent[node].iter().filter_map(move |&j| {
            let e = &self.edges[j];
            let enabled = if e.membership {
                membership
            } else {
                relationships
            };
            enabled.then_some(if e.from == node { e.to } else { e.from })
        })
    }

    pub(super) fn visible(
        &self,
        hidden: &BTreeSet<Layer>,
        focus: Option<usize>,
        membership: bool,
        relationships: bool,
    ) -> Vec<bool> {
        let mut shown = vec![focus.is_none(); self.nodes.len()];
        if let Some(i) = focus {
            shown[i] = true;
            for neighbour in self.neighbours(i, membership, relationships) {
                shown[neighbour] = true;
            }
        }
        for (show, dot) in shown.iter_mut().zip(&self.nodes) {
            *show &= !hidden.contains(&dot.layer);
        }
        shown
    }
}
