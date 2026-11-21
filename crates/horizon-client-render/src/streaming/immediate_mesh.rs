//! Synchronous section meshing for individual block edits (`World::immediate_sections`).
//!
//! Vanilla's `LevelRenderer.compileSections` / `SectionRenderDispatcher.rebuildSectionSync`
//! rebuilds player-edited sections on the render thread before drawing the frame so a placed
//! or broken block appears on the same frame as its selection outline instead of lagging one
//! or more frames behind on worker threads.
use super::{INLINE_RELIGHT_MAX, Streaming, mesh_section};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    mesher::SectionMesh,
    section_mask::SectionMask,
    world::{DirtySections, World},
};

/// Maximum number of sections rebuilt synchronously in one frame.
///
/// A single block edit on a chunk/section corner touches at most 8 sections; larger bursts
/// (bulk updates or scripts calling `set_block` in a loop) fall back to the background worker
/// pool via `World::dirty_sections` so they never stall a frame.
pub const IMMEDIATE_MESH_LIMIT: usize = 8;

/// Take ready columns from `immediate` within `IMMEDIATE_MESH_LIMIT`, removing accepted sections
/// from `dirty`.
fn take_ready_immediate_sections(
    immediate: &mut DirtySections,
    dirty: &mut DirtySections,
    lit: &FxHashSet<(i32, i32)>,
    lighting: &FxHashMap<(i32, i32), u64>,
    in_view: impl Fn(i32, i32) -> bool,
    has_chunk: impl Fn(i32, i32) -> bool,
) -> Vec<((i32, i32), SectionMask)> {
    let total = immediate.len();
    if total == 0 || total > IMMEDIATE_MESH_LIMIT {
        immediate.clear();
        return Vec::new();
    }
    let taken = std::mem::take(immediate);
    let mut ready_columns = Vec::new();
    for ((x, z), sections) in taken.columns() {
        if !has_chunk(x, z) || !in_view(x, z) {
            continue;
        }
        let neighbours_ready = (-1..=1).all(|dz| {
            (-1..=1).all(|dx| {
                let n = (x + dx, z + dz);
                lit.contains(&n) && !lighting.contains_key(&n)
            })
        });
        if !neighbours_ready {
            continue;
        }
        let mut accepted = SectionMask::EMPTY;
        for sy in sections.iter().map(|sy| sy as i32) {
            // Remove from `dirty` if still queued; even if streaming already removed it to
            // dispatch a worker job, accept it so the synchronous rebuild supersedes the
            // in-flight worker job.
            dirty.remove((x, sy, z));
            accepted.insert(sy as usize);
        }
        if !accepted.is_empty() {
            ready_columns.push(((x, z), accepted));
        }
    }
    ready_columns
}

impl Streaming {
    /// Synchronously rebuild sections dirtied by individual block edits since `update_world`,
    /// returning meshes ready for immediate GPU upload before the current frame draws.
    pub fn mesh_immediate(&mut self, world: &mut World) -> Vec<((i32, i32, i32), SectionMesh)> {
        if !self.mesh || world.immediate_sections.is_empty() {
            world.immediate_sections.clear();
            return Vec::new();
        }
        if !world.light_edits.is_empty() {
            if world.light_edits.len() > INLINE_RELIGHT_MAX {
                // Pending lighting exceeds the inline budget: leave all affected sections dirty
                // for the normal relighting and worker meshing path instead of uploading stale light.
                for ((x, z), sections) in std::mem::take(&mut world.immediate_sections).columns() {
                    for sy in sections.iter().map(|sy| sy as i32) {
                        world.dirty_sections.insert((x, sy, z));
                    }
                }
                return Vec::new();
            }
            self.relight_edits(world);
        }
        let ready_columns = {
            let ranges = self.ranges();
            let chunks = &world.chunks;
            take_ready_immediate_sections(
                &mut world.immediate_sections,
                &mut world.dirty_sections,
                &self.lit,
                &self.lighting,
                |x, z| ranges.in_view(x, z),
                |x, z| chunks.contains_key(&(x, z)),
            )
        };
        if ready_columns.is_empty() {
            return Vec::new();
        }
        let context = self.shared.context.read().unwrap().clone();
        let settings = self.mesh_settings();
        let mut uploads = Vec::new();
        for ((x, z), sections) in ready_columns {
            let Some(region) = Self::region(world, x, z) else {
                // Keep sections dirty for workers if a neighbour column disappeared.
                for sy in sections.iter().map(|sy| sy as i32) {
                    world.dirty_sections.insert((x, sy, z));
                }
                continue;
            };
            self.meshed.insert((x, z));
            for sy in sections.iter().map(|sy| sy as i32) {
                // Invalidate any older in-flight worker job for this section so its stale
                // `Done::Meshed` is discarded when it finishes.
                self.meshing.remove(&(x, sy, z));
                let recycled = self.shared.meshes.lock().unwrap().take();
                let mesh = mesh_section(
                    &context,
                    &settings,
                    &mut self.immediate_faces,
                    &mut self.immediate_scratch,
                    &region,
                    sy,
                    recycled,
                );
                self.sequence += 1;
                self.compile_started.insert((x, sy, z), self.sequence);
                self.meshed_total += 1;
                uploads.push(((x, sy, z), mesh));
            }
        }
        uploads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    #[test]
    fn dirty_sections_remove_clears_single_section_and_prunes_empty_column() {
        let mut dirty = DirtySections::default();
        dirty.insert((1, 4, -2));
        dirty.insert((1, 5, -2));
        assert!(dirty.remove((1, 4, -2)));
        assert!(!dirty.remove((1, 4, -2)));
        assert!(!dirty.contains(&(1, 4, -2)));
        assert!(dirty.contains(&(1, 5, -2)));
        assert_eq!(dirty.len(), 1);
        assert!(dirty.remove((1, 5, -2)));
        assert!(dirty.is_empty());
        assert_eq!(dirty.columns().count(), 0);
    }

    #[test]
    fn set_block_marks_immediate_sections_for_edit_and_neighbours() {
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.set(8, 64, 8, BlockStateId(1)).unwrap();
        let sy = (64 - world.bottom_y()) >> 4;
        assert!(world.immediate_sections.contains(&(0, sy, 0)));
        assert!(world.dirty_sections.contains(&(0, sy, 0)));
        // Corner of a section marks up to 8 sections (within IMMEDIATE_MESH_LIMIT).
        world.immediate_sections.clear();
        world.dirty_sections.clear();
        world.set(0, 64, 0, BlockStateId(2)).unwrap();
        assert_eq!(world.immediate_sections.len(), 8);
        assert!(world.immediate_sections.len() <= IMMEDIATE_MESH_LIMIT);
    }

    #[test]
    fn take_ready_immediate_clears_dirty_and_keeps_other_sections() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        immediate.insert((0, 4, 0));
        dirty.insert((0, 4, 0));
        dirty.insert((0, 5, 0));

        let ready = take_ready_immediate_sections(
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, (0, 0));
        assert!(ready[0].1.contains(4));
        assert!(immediate.is_empty());
        assert!(!dirty.contains(&(0, 4, 0)));
        assert!(dirty.contains(&(0, 5, 0)));
    }

    #[test]
    fn take_ready_immediate_falls_back_to_workers_above_limit() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        for sy in 0..=(IMMEDIATE_MESH_LIMIT as i32) {
            immediate.insert((0, sy, 0));
            dirty.insert((0, sy, 0));
        }

        let ready = take_ready_immediate_sections(
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert!(ready.is_empty());
        assert!(immediate.is_empty());
        assert_eq!(dirty.len(), IMMEDIATE_MESH_LIMIT + 1);
    }

    #[test]
    fn take_ready_immediate_skips_columns_with_unlit_or_lighting_neighbours() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let mut lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        lighting.insert((1, 0), 7);
        immediate.insert((0, 4, 0));
        dirty.insert((0, 4, 0));

        let ready = take_ready_immediate_sections(
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert!(ready.is_empty());
        assert!(immediate.is_empty());
        assert!(dirty.contains(&(0, 4, 0)));
    }

    #[test]
    fn take_ready_immediate_accepts_sections_already_dispatched_to_workers() {
        let mut immediate = DirtySections::default();
        let mut dirty = DirtySections::default();
        let mut lit = FxHashSet::default();
        let lighting = FxHashMap::default();
        for dz in -1..=1 {
            for dx in -1..=1 {
                lit.insert((dx, dz));
            }
        }
        // Simulate an edit whose dirty entry was already consumed by streaming worker dispatch.
        immediate.insert((0, 4, 0));

        let ready = take_ready_immediate_sections(
            &mut immediate,
            &mut dirty,
            &lit,
            &lighting,
            |_, _| true,
            |_, _| true,
        );
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, (0, 0));
        assert!(ready[0].1.contains(4));
        assert!(immediate.is_empty());
    }
}
