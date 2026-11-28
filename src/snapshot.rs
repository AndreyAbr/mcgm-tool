use crate::anvil::WorldCoords;
use crate::atlas::TexRequestKey;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub const SNAPSHOT_COORD_SYSTEM: &str = "mc_world_y_up_rhs";
pub const SNAPSHOT_TEX_PREFIX: &str = "mcgm_snapshot:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl SnapshotBounds {
    pub fn is_valid(&self) -> bool {
        self.min_x <= self.max_x && self.min_y <= self.max_y && self.min_z <= self.max_z
    }

    pub fn intersects_world_coords(&self, c: &WorldCoords) -> bool {
        let nc = c.normalized();
        self.min_x <= nc.max_x
            && self.max_x >= nc.min_x
            && self.min_y <= nc.max_y
            && self.max_y >= nc.min_y
            && self.min_z <= nc.max_z
            && self.max_z >= nc.min_z
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotSummaryMeta {
    pub captured_blocks: usize,
    pub full_blocks: usize,
    pub partial_blocks: usize,
    pub fallback_blocks: usize,
    pub unique_meshes: usize,
    pub unique_textures: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTextureMeta {
    pub id: String,
    pub path: String,
    pub source_id: String,
    pub width: u32,
    pub height: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeshMeta {
    pub id: String,
    pub path: String,
    pub triangle_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureStatus {
    Full,
    Partial,
    Fallback,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotBlockEntry {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block_id: String,
    #[serde(default)]
    pub has_block_entity: bool,
    pub capture_status: CaptureStatus,
    #[serde(default)]
    pub capture_reason: Option<String>,
    #[serde(default)]
    pub mesh_id: Option<String>,
    #[serde(default)]
    pub has_collision: bool,
    #[serde(default)]
    pub collision_boxes: Vec<[f64; 6]>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub format_version: u32,
    pub coord_system: String,
    pub dimension: String,
    #[serde(default)]
    pub world_name: String,
    #[serde(default)]
    pub captured_at_utc: String,
    pub bounds: SnapshotBounds,
    #[serde(default)]
    pub summary: SnapshotSummaryMeta,
    #[serde(default)]
    pub textures: Vec<SnapshotTextureMeta>,
    #[serde(default)]
    pub meshes: Vec<SnapshotMeshMeta>,
    #[serde(default)]
    pub blocks: Vec<SnapshotBlockEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotRenderLayer {
    Solid,
    Cutout,
    Translucent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTri {
    pub pos: [[f32; 3]; 3],
    pub normal: [f32; 3],
    pub uv: [[f32; 2]; 3],
    pub texture_id: String,
    pub render_layer: SnapshotRenderLayer,
    #[serde(default)]
    pub tint_rgb: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeshFile {
    pub id: String,
    pub triangles: Vec<SnapshotTri>,
}

#[derive(Debug, Clone, Default)]
pub struct SnapshotApplyReport {
    #[allow(dead_code)]
    pub total_manifest_blocks: usize,
    pub covered_blocks: usize,
    pub full_blocks: usize,
    pub partial_blocks: usize,
    pub fallback_blocks: usize,
    pub unique_meshes: usize,
    pub unique_textures: usize,
    pub partial_or_fallback_samples: Vec<String>,
}

impl SnapshotApplyReport {
    pub fn format_log_line(&self) -> String {
        format!(
            "Mod snapshot applied: {} covered ({} full, {} partial), {} fallback, {} meshes, {} textures",
            self.covered_blocks,
            self.full_blocks,
            self.partial_blocks,
            self.fallback_blocks,
            self.unique_meshes,
            self.unique_textures
        )
    }
}

#[derive(Debug, Clone)]
pub struct LoadedModSnapshot {
    #[allow(dead_code)]
    pub root_dir: PathBuf,
    pub manifest: SnapshotManifest,
    pub textures_png: HashMap<String, Vec<u8>>,
    pub meshes: HashMap<String, Vec<SnapshotTri>>,
    pub blocks_by_pos: HashMap<(i32, i32, i32), usize>,
    pub report: SnapshotApplyReport,
}

#[inline]
pub fn snapshot_tex_ref(tex_id: &str) -> String {
    format!("{}{}", SNAPSHOT_TEX_PREFIX, tex_id)
}

pub fn normalize_snapshot_dimension(dim: &str) -> &'static str {
    let d = dim.trim().to_ascii_lowercase();
    match d.as_str() {
        "overworld" | "minecraft:overworld" => "minecraft:overworld",
        "nether" | "the_nether" | "minecraft:the_nether" => "minecraft:the_nether",
        "end" | "the_end" | "minecraft:the_end" => "minecraft:the_end",
        _ => "custom",
    }
}

fn resolve_safe_relative_file(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.trim().is_empty() {
        return Err("Empty relative path inside mod snapshot manifest".to_string());
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(format!("Snapshot path must be relative, got '{}'", rel));
    }
    for comp in rel_path.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Unsafe snapshot relative path '{}'", rel));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    let full = root.join(rel_path);
    if !full.is_file() {
        return Err(format!(
            "Missing referenced snapshot file: {}",
            full.display()
        ));
    }
    Ok(full)
}

impl LoadedModSnapshot {
    pub fn load_and_validate(
        path: &Path,
        export_coords: &WorldCoords,
        export_dimension: &str,
    ) -> Result<Self, String> {
        let (root_dir, manifest_path) = if path.is_file() {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            (parent, path.to_path_buf())
        } else if path.is_dir() {
            (path.to_path_buf(), path.join("manifest.json"))
        } else {
            return Err(format!(
                "Mod snapshot path does not exist: {}",
                path.display()
            ));
        };

        if !manifest_path.is_file() {
            return Err(format!(
                "Snapshot manifest.json not found at {}",
                manifest_path.display()
            ));
        }

        let manifest_text = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read {}: {}", manifest_path.display(), e))?;
        let manifest: SnapshotManifest = serde_json::from_str(&manifest_text)
            .map_err(|e| format!("Invalid snapshot manifest JSON: {}", e))?;

        if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(format!(
                "Unsupported mod snapshot format_version {} (expected {})",
                manifest.format_version, SNAPSHOT_FORMAT_VERSION
            ));
        }
        if manifest.coord_system != SNAPSHOT_COORD_SYSTEM {
            return Err(format!(
                "Unsupported mod snapshot coord_system '{}' (expected '{}')",
                manifest.coord_system, SNAPSHOT_COORD_SYSTEM
            ));
        }

        let norm_snap_dim = normalize_snapshot_dimension(&manifest.dimension);
        let norm_exp_dim = normalize_snapshot_dimension(export_dimension);
        let dims_match = if norm_snap_dim == "custom" || norm_exp_dim == "custom" {
            manifest
                .dimension
                .trim()
                .eq_ignore_ascii_case(export_dimension.trim())
        } else {
            norm_snap_dim == norm_exp_dim
        };
        if !dims_match {
            return Err(format!(
                "Mod snapshot dimension '{}' does not match export dimension '{}'",
                manifest.dimension, export_dimension
            ));
        }

        if !manifest.bounds.is_valid() {
            return Err(format!(
                "Invalid mod snapshot bounds ({}, {}, {})..({}, {}, {})",
                manifest.bounds.min_x,
                manifest.bounds.min_y,
                manifest.bounds.min_z,
                manifest.bounds.max_x,
                manifest.bounds.max_y,
                manifest.bounds.max_z
            ));
        }

        let norm_export = export_coords.normalized();
        if !manifest.bounds.intersects_world_coords(&norm_export) {
            return Err(format!(
                "Mod snapshot bounds ({}, {}, {})..({}, {}, {}) do not intersect export bounds ({}, {}, {})..({}, {}, {})",
                manifest.bounds.min_x,
                manifest.bounds.min_y,
                manifest.bounds.min_z,
                manifest.bounds.max_x,
                manifest.bounds.max_y,
                manifest.bounds.max_z,
                norm_export.min_x,
                norm_export.min_y,
                norm_export.min_z,
                norm_export.max_x,
                norm_export.max_y,
                norm_export.max_z
            ));
        }

        let mut textures_png = HashMap::with_capacity(manifest.textures.len());
        for tm in &manifest.textures {
            let full = resolve_safe_relative_file(&root_dir, &tm.path)?;
            let bytes = std::fs::read(&full)
                .map_err(|e| format!("Failed to read snapshot texture {}: {}", full.display(), e))?;
            if bytes.is_empty() {
                return Err(format!("Snapshot texture file is empty: {}", full.display()));
            }
            textures_png.insert(tm.id.clone(), bytes);
        }

        let mut meshes = HashMap::with_capacity(manifest.meshes.len());
        for mm in &manifest.meshes {
            let full = resolve_safe_relative_file(&root_dir, &mm.path)?;
            let text = std::fs::read_to_string(&full)
                .map_err(|e| format!("Failed to read snapshot mesh {}: {}", full.display(), e))?;
            let mf: SnapshotMeshFile = serde_json::from_str(&text)
                .map_err(|e| format!("Invalid snapshot mesh JSON {}: {}", full.display(), e))?;
            if mf.id != mm.id {
                return Err(format!(
                    "Snapshot mesh id mismatch in {}: manifest says '{}', file says '{}'",
                    full.display(),
                    mm.id,
                    mf.id
                ));
            }
            for tri in &mf.triangles {
                if !textures_png.contains_key(&tri.texture_id) {
                    return Err(format!(
                        "Snapshot mesh '{}' references unknown texture_id '{}'",
                        mf.id, tri.texture_id
                    ));
                }
            }
            meshes.insert(mm.id.clone(), mf.triangles);
        }

        let mut blocks_by_pos = HashMap::new();
        let mut report = SnapshotApplyReport {
            total_manifest_blocks: manifest.blocks.len(),
            unique_meshes: meshes.len(),
            unique_textures: textures_png.len(),
            ..Default::default()
        };

        for (idx, b) in manifest.blocks.iter().enumerate() {
            if let Some(ref mid) = b.mesh_id {
                if !meshes.contains_key(mid) {
                    return Err(format!(
                        "Snapshot block {} at ({}, {}, {}) references missing mesh_id '{}'",
                        b.block_id, b.x, b.y, b.z, mid
                    ));
                }
            }

            let in_bounds = b.x >= norm_export.min_x
                && b.x <= norm_export.max_x
                && b.y >= norm_export.min_y
                && b.y <= norm_export.max_y
                && b.z >= norm_export.min_z
                && b.z <= norm_export.max_z;
            if !in_bounds {
                continue;
            }

            blocks_by_pos.insert((b.x, b.y, b.z), idx);
            match b.capture_status {
                CaptureStatus::Full => {
                    report.covered_blocks += 1;
                    report.full_blocks += 1;
                }
                CaptureStatus::Partial => {
                    report.covered_blocks += 1;
                    report.partial_blocks += 1;
                    if report.partial_or_fallback_samples.len() < 16 {
                        report.partial_or_fallback_samples.push(format!(
                            "{} @ ({}, {}, {}) partial{}",
                            b.block_id,
                            b.x,
                            b.y,
                            b.z,
                            b.capture_reason
                                .as_deref()
                                .map(|r| format!(": {}", r))
                                .unwrap_or_default()
                        ));
                    }
                }
                CaptureStatus::Fallback => {
                    report.fallback_blocks += 1;
                    if report.partial_or_fallback_samples.len() < 16 {
                        report.partial_or_fallback_samples.push(format!(
                            "{} @ ({}, {}, {}) fallback{}",
                            b.block_id,
                            b.x,
                            b.y,
                            b.z,
                            b.capture_reason
                                .as_deref()
                                .map(|r| format!(": {}", r))
                                .unwrap_or_default()
                        ));
                    }
                }
            }
        }

        Ok(Self {
            root_dir,
            manifest,
            textures_png,
            meshes,
            blocks_by_pos,
            report,
        })
    }

    pub fn collect_tex_requests(&self) -> Vec<TexRequestKey> {
        let mut used_meshes = HashSet::new();
        for &idx in self.blocks_by_pos.values() {
            let b = &self.manifest.blocks[idx];
            if b.capture_status != CaptureStatus::Fallback {
                if let Some(ref mid) = b.mesh_id {
                    used_meshes.insert(mid.as_str());
                }
            }
        }

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (mid, tris) in &self.meshes {
            if !used_meshes.contains(mid.as_str()) {
                continue;
            }
            for tri in tris {
                let key = TexRequestKey {
                    tex_ref: snapshot_tex_ref(&tri.texture_id),
                    tint_rgb: tri.tint_rgb,
                };
                if seen.insert(key.clone()) {
                    out.push(key);
                }
            }
        }
        out
    }

    pub fn register_textures(&self, assets: &mut crate::assets::AssetIndex) {
        for (tex_id, png_bytes) in &self.textures_png {
            let key = snapshot_tex_ref(tex_id);
            assets.register_snapshot_texture(&key, png_bytes.clone());
        }
    }

    /// Returns Some(entry) if the snapshot covers (wx, wy, wz) and is NOT in Fallback status.
    #[inline]
    pub fn covered_block(&self, wx: i32, wy: i32, wz: i32) -> Option<&SnapshotBlockEntry> {
        let &idx = self.blocks_by_pos.get(&(wx, wy, wz))?;
        let b = &self.manifest.blocks[idx];
        if b.capture_status == CaptureStatus::Fallback {
            None
        } else {
            Some(b)
        }
    }

    #[inline]
    pub fn is_covered_block(&self, wx: i32, wy: i32, wz: i32) -> bool {
        self.covered_block(wx, wy, wz).is_some()
    }
}
