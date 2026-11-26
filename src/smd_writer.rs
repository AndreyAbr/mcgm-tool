use crate::collision::CollisionBox;
use crate::mesher::{ChunkMesh, MeshTri};
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const MAX_VERTS_PER_SMD: usize = 28_000;
pub const MAX_TRIS_PER_SMD: usize = MAX_VERTS_PER_SMD / 3; // 9,333 tris = 27,999 vertices <= 28,000

const BOX_TRIS: [[usize; 3]; 12] = [
    [4, 5, 6],
    [4, 6, 7], // +Z Top
    [0, 3, 2],
    [0, 2, 1], // -Z Bottom
    [1, 2, 6],
    [1, 6, 5], // +X East
    [0, 4, 7],
    [0, 7, 3], // -X West
    [3, 7, 6],
    [3, 6, 2], // +Y North
    [0, 1, 5],
    [0, 5, 4], // -Y South
];

#[derive(Debug, Clone)]
pub struct CompiledChunkArtifact {
    pub chunk_name: String,
    pub origin_source: [f64; 3],
    pub qc_path: PathBuf,
    pub model_rel_path: String,
    pub translucent_qc_path: Option<PathBuf>,
    pub translucent_model_rel_path: Option<String>,
    pub extra_collision_models: Vec<(PathBuf, String)>,
    pub extra_collision_boxes: Vec<Vec<CollisionBox>>,
    pub overflow_boxes: Vec<CollisionBox>,
    pub fallback_vmf_brushes: bool,
}

pub fn write_visual_smd_parts(
    tris: &[MeshTri],
    out_dir: &Path,
    file_prefix: &str,
    origin_source: [f64; 3],
) -> Result<Vec<PathBuf>, String> {
    if tris.is_empty() {
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir failed: {}", e))?;

    let mut paths = Vec::new();
    let ox = origin_source[0] as f32;
    let oy = origin_source[1] as f32;
    let oz = origin_source[2] as f32;

    let mut sorted_tris: Vec<&MeshTri> = tris.iter().collect();
    sorted_tris.sort_by(|a, b| a.material.cmp(&b.material));

    for (part_idx, chunk_slice) in sorted_tris.chunks(MAX_TRIS_PER_SMD).enumerate() {
        let filename = format!("{}_part_{}.smd", file_prefix, part_idx);
        let full_path = out_dir.join(&filename);
        let file = File::create(&full_path)
            .map_err(|e| format!("Failed creating {}: {}", full_path.display(), e))?;
        let mut w = BufWriter::with_capacity(256 * 1024, file);

        writeln!(
            w,
            "version 1\nnodes\n0 \"root\" -1\nend\nskeleton\ntime 0\n0 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000\nend\ntriangles"
        )
        .map_err(|e| e.to_string())?;

        for tri in chunk_slice {
            writeln!(w, "{}", tri.material).map_err(|e| e.to_string())?;
            for vi in 0..3 {
                let lx = tri.pos[vi][0] - ox;
                let ly = tri.pos[vi][1] - oy;
                let lz = tri.pos[vi][2] - oz;
                // Pre-compensate StudioMDL +90 deg Z rotation: SMD = (ly, -lx, lz)
                let nx = tri.norm[vi][0];
                let ny = tri.norm[vi][1];
                let nz = tri.norm[vi][2];
                writeln!(
                    w,
                    "0 {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}",
                    ly, -lx, lz, ny, -nx, nz, tri.uv[vi][0], tri.uv[vi][1]
                )
                .map_err(|e| e.to_string())?;
            }
        }
        writeln!(w, "end").map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        paths.push(full_path);
    }

    Ok(paths)
}

fn collision_box_priority(b: &CollisionBox) -> f64 {
    let coords = b.box_coords;
    let z_max = coords[2].max(coords[5]);
    let vol = ((coords[3] - coords[0]) * (coords[4] - coords[1]) * (coords[5] - coords[2])).abs();
    if b.block_class == "leaves" {
        7_000_000.0 + z_max
    } else if matches!(
        b.block_class.as_ref(),
        "snow" | "slab" | "stairs" | "trapdoor"
    ) {
        12_500_000.0 + z_max
    } else if matches!(b.block_class.as_ref(), "wall" | "fence" | "pane") {
        8_000_000.0 + z_max
    } else if b.is_walkable {
        12_000_000.0 + z_max
    } else if b.block_class == "full" || b.block_class == "glass" {
        6_000_000.0 + z_max
    } else {
        100_000.0 + z_max * 10.0 - (vol / 1000.0).min(50_000.0)
    }
}

fn sort_collision_box_refs_by_priority(boxes: &mut [&CollisionBox]) {
    boxes.sort_by(|a, b| {
        let pa = collision_box_priority(a);
        let pb = collision_box_priority(b);
        pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
fn sort_collision_boxes_by_priority<'a>(boxes: &'a [CollisionBox]) -> Vec<&'a CollisionBox> {
    let mut refs: Vec<&'a CollisionBox> = boxes.iter().collect();
    sort_collision_box_refs_by_priority(&mut refs);
    refs
}

fn write_physics_smd_refs(
    active_boxes: &[&CollisionBox],
    smd_path: &Path,
    chunk_center: [f64; 3],
    epsilon: f64,
) -> Result<(), String> {
    let file = File::create(smd_path)
        .map_err(|e| format!("Failed creating {}: {}", smd_path.display(), e))?;
    let mut w = BufWriter::with_capacity(128 * 1024, file);

    writeln!(
        w,
        "version 1\nnodes\n0 \"root\" -1\nend\nskeleton\ntime 0\n0 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000\nend\ntriangles"
    )
    .map_err(|e| e.to_string())?;

    let cc_x = chunk_center[0];
    let cc_y = chunk_center[1];
    let cc_z = chunk_center[2];
    let mut v_lines: [String; 8] = std::array::from_fn(|_| String::with_capacity(96));

    for &b in active_boxes {
        let c = b.box_coords;
        let x_min = c[0].min(c[3]) - cc_x;
        let x_max = c[0].max(c[3]) - cc_x;
        let y_min = c[1].min(c[4]) - cc_y;
        let y_max = c[1].max(c[4]) - cc_y;
        let z_min = c[2].min(c[5]) - cc_z;
        let z_max = c[2].max(c[5]) - cc_z;

        let dx = x_max - x_min;
        let dy = y_max - y_min;
        let dz = z_max - z_min;

        let eps_x = epsilon.min(dx * 0.4);
        let eps_y = epsilon.min(dy * 0.4);
        let eps_z_bot = epsilon.min(dz * 0.4);
        let eps_z_top = (0.002f64).min(epsilon).min(dz * 0.2);

        let mut bx0 = x_min + eps_x;
        let mut bx1 = x_max - eps_x;
        let mut by0 = y_min + eps_y;
        let mut by1 = y_max - eps_y;
        let mut bz0 = z_min + eps_z_bot;
        let mut bz1 = z_max - eps_z_top;

        if (bx1 - bx0) < 0.6 {
            let mid = (bx0 + bx1) * 0.5;
            bx0 = mid - 0.3;
            bx1 = mid + 0.3;
        }
        if (by1 - by0) < 0.6 {
            let mid = (by0 + by1) * 0.5;
            by0 = mid - 0.3;
            by1 = mid + 0.3;
        }
        if (bz1 - bz0) < 0.6 {
            let mid = (bz0 + bz1) * 0.5;
            bz0 = mid - 0.3;
            bz1 = mid + 0.3;
        }

        let cx = (bx0 + bx1) * 0.5;
        let cy = (by0 + by1) * 0.5;
        let cz = (bz0 + bz1) * 0.5;

        let corners = [
            [bx0, by0, bz0],
            [bx1, by0, bz0],
            [bx1, by1, bz0],
            [bx0, by1, bz0],
            [bx0, by0, bz1],
            [bx1, by0, bz1],
            [bx1, by1, bz1],
            [bx0, by1, bz1],
        ];
        for (idx, [vx, vy, vz]) in corners.into_iter().enumerate() {
            let nx = vx - cx;
            let ny = vy - cy;
            let nz = vz - cz;
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            v_lines[idx].clear();
            let _ = write!(
                v_lines[idx],
                "0 {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} 0.000000 0.000000",
                vy,
                -vx,
                vz,
                ny / len,
                -nx / len,
                nz / len
            );
        }

        for [i1, i2, i3] in BOX_TRIS {
            w.write_all(b"phys\n").map_err(|e| e.to_string())?;
            w.write_all(v_lines[i1].as_bytes()).map_err(|e| e.to_string())?;
            w.write_all(b"\n").map_err(|e| e.to_string())?;
            w.write_all(v_lines[i2].as_bytes()).map_err(|e| e.to_string())?;
            w.write_all(b"\n").map_err(|e| e.to_string())?;
            w.write_all(v_lines[i3].as_bytes()).map_err(|e| e.to_string())?;
            w.write_all(b"\n").map_err(|e| e.to_string())?;
        }
    }

    w.write_all(b"end\n").map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
pub fn write_physics_smd(
    boxes: &[CollisionBox],
    smd_path: &Path,
    chunk_center: [f64; 3],
    max_pieces: usize,
    epsilon: f64,
) -> Result<Vec<CollisionBox>, String> {
    if let Some(parent) = smd_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }

    if boxes.len() > max_pieces {
        let sorted = sort_collision_boxes_by_priority(boxes);
        write_physics_smd_refs(&sorted[..max_pieces], smd_path, chunk_center, epsilon)?;
        Ok(sorted[max_pieces..].iter().map(|&b| b.clone()).collect())
    } else {
        let refs: Vec<&CollisionBox> = boxes.iter().collect();
        write_physics_smd_refs(&refs, smd_path, chunk_center, epsilon)?;
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn write_qc_file(
    qc_path: &Path,
    category: &str,
    model_stem: &str,
    smd_filenames: &[String],
    phys_smd_filename: Option<&str>,
    mostly_opaque: bool,
    cast_shadows: bool,
    max_convex_pieces: usize,
) -> Result<(), String> {
    write_qc_file_with_options(
        qc_path,
        category,
        model_stem,
        smd_filenames,
        phys_smd_filename,
        mostly_opaque,
        false,
        cast_shadows,
        max_convex_pieces,
        "dirt",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn write_qc_file_with_options(
    qc_path: &Path,
    category: &str,
    model_stem: &str,
    smd_filenames: &[String],
    phys_smd_filename: Option<&str>,
    mostly_opaque: bool,
    translucent_only: bool,
    cast_shadows: bool,
    max_convex_pieces: usize,
    surface_prop: &str,
) -> Result<(), String> {
    if smd_filenames.is_empty() {
        return Err("No SMD parts for QC file".to_string());
    }
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let surf = if surface_prop.trim().is_empty() {
        "dirt"
    } else {
        surface_prop.trim()
    };
    let mut lines = Vec::new();
    lines.push(format!("$modelname \"{}/{}.mdl\"", clean_cat, model_stem));
    lines.push(format!("$cdmaterials \"models/{}\"", clean_cat));
    lines.push(format!("$surfaceprop \"{}\"", surf));
    lines.push("$staticprop".to_string());
    if cast_shadows {
        lines.push("$casttextureshadows".to_string());
    }
    lines.push("$origin 0 0 0 0".to_string());
    lines.push("$illumposition 0 0 0".to_string());
    if !translucent_only {
        if mostly_opaque {
            lines.push("$mostlyopaque".to_string());
        } else {
            lines.push("$opaque".to_string());
        }
    }
    for (idx, smd) in smd_filenames.iter().enumerate() {
        lines.push(format!("$body \"chunk_{}\" \"{}\"", idx, smd));
    }
    lines.push(format!(
        "$sequence \"idle\" \"{}\" {{\n    fps 1\n}}",
        smd_filenames[0]
    ));
    if let Some(phys) = phys_smd_filename {
        lines.push(format!(
            "$collisionmodel \"{}\" {{\n    $concave\n    $automass\n    $maxconvexpieces {}\n}}",
            phys, max_convex_pieces
        ));
    }
    std::fs::write(qc_path, lines.join("\n") + "\n")
        .map_err(|e| format!("Failed writing QC {}: {}", qc_path.display(), e))
}

pub fn emit_chunk_smds_and_qcs(
    chunk: &ChunkMesh,
    collision_boxes: &[CollisionBox],
    out_dir: &Path,
    category: &str,
    max_convex_pieces: usize,
    export_name: &str,
) -> Result<CompiledChunkArtifact, String> {
    let clean_export: String = export_name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean_export = clean_export.trim_matches('_');
    let model_stem = if clean_export.is_empty() || chunk.name.starts_with(&format!("{}_", clean_export)) {
        chunk.name.clone()
    } else {
        format!("{}_{}", clean_export, chunk.name)
    };

    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir failed: {}", e))?;

    let mut solid_filenames = Vec::new();

    if !chunk.solid_tris.is_empty() {
        let paths = write_visual_smd_parts(
            &chunk.solid_tris,
            out_dir,
            &model_stem,
            chunk.origin_source,
        )?;
        for p in &paths {
            if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
                solid_filenames.push(fname.to_string());
            }
        }
    }

    let mut phys_filename: Option<String> = None;
    let mut primary_surface_prop = "rock";
    let mut overflow_boxes = Vec::new();
    let mut extra_collision_models: Vec<(PathBuf, String)> = Vec::new();
    let mut extra_collision_boxes: Vec<Vec<CollisionBox>> = Vec::new();
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');

    if !collision_boxes.is_empty() {
        let mut by_surface: std::collections::BTreeMap<crate::collision::SurfaceProp, Vec<&CollisionBox>> =
            std::collections::BTreeMap::new();
        for b in collision_boxes {
            by_surface.entry(b.surface_prop).or_default().push(b);
        }

        let mut remaining_groups: Vec<(crate::collision::SurfaceProp, Vec<&CollisionBox>)> = Vec::new();

        if !solid_filenames.is_empty() {
            // Pick the largest surface group for the main model's embedded physics hull.
            let primary_sp = by_surface
                .iter()
                .max_by_key(|(sp, vec)| (vec.len(), std::cmp::Reverse(*sp)))
                .map(|(sp, _)| *sp)
                .unwrap();
            primary_surface_prop = primary_sp.as_source_str();

            if let Some(mut primary_boxes) = by_surface.remove(&primary_sp) {
                if max_convex_pieces > 0 && primary_boxes.len() > max_convex_pieces {
                    sort_collision_box_refs_by_priority(&mut primary_boxes);
                }
                let take_n = if max_convex_pieces > 0 {
                    max_convex_pieces.min(primary_boxes.len())
                } else {
                    primary_boxes.len()
                };
                let phys_name = format!("{}_phys.smd", model_stem);
                let p_path = out_dir.join(&phys_name);
                write_physics_smd_refs(&primary_boxes[..take_n], &p_path, chunk.origin_source, 0.015)?;
                phys_filename = Some(phys_name);
                if take_n < primary_boxes.len() {
                    remaining_groups.push((primary_sp, primary_boxes[take_n..].to_vec()));
                }
            }
        }

        for (sp, boxes_for_sp) in by_surface {
            remaining_groups.push((sp, boxes_for_sp));
        }

        let rem_total: usize = remaining_groups.iter().map(|(_, v)| v.len()).sum();
        if rem_total > 0 && max_convex_pieces > 0 && (!solid_filenames.is_empty() || collision_boxes.len() > 64) {
            let ref_smd_name = format!("{}_col_ref.smd", model_stem);
            let ref_smd_path = out_dir.join(&ref_smd_name);
            write_dummy_collision_ref_smd(&ref_smd_path)?;

            for (sp, mut boxes_for_sp) in remaining_groups {
                let surf = sp.as_source_str();
                boxes_for_sp.sort_by(|a, b| {
                    let acx = a.box_coords[0] + a.box_coords[3];
                    let bcx = b.box_coords[0] + b.box_coords[3];
                    let acy = a.box_coords[1] + a.box_coords[4];
                    let bcy = b.box_coords[1] + b.box_coords[4];
                    let acz = a.box_coords[2] + a.box_coords[5];
                    let bcz = b.box_coords[2] + b.box_coords[5];
                    acx.total_cmp(&bcx)
                        .then_with(|| acy.total_cmp(&bcy))
                        .then_with(|| acz.total_cmp(&bcz))
                });
                for (idx, batch) in boxes_for_sp.chunks(max_convex_pieces).enumerate() {
                    let col_stem = format!("{}_col_{}_{}", model_stem, surf, idx + 1);
                    let col_phys_name = format!("{}_phys.smd", col_stem);
                    let col_phys_path = out_dir.join(&col_phys_name);
                    write_physics_smd_refs(batch, &col_phys_path, chunk.origin_source, 0.015)?;
                    let col_qc_path = out_dir.join(format!("{}.qc", col_stem));
                    write_collision_only_qc(
                        &col_qc_path,
                        clean_cat,
                        &col_stem,
                        &ref_smd_name,
                        &col_phys_name,
                        surf,
                        max_convex_pieces,
                    )?;
                    extra_collision_models.push((
                        col_qc_path,
                        format!("models/{}/{}.mdl", clean_cat, col_stem),
                    ));
                    extra_collision_boxes.push(batch.iter().map(|&b| b.clone()).collect());
                }
            }
        } else if rem_total > 0 {
            for (_, boxes_for_sp) in remaining_groups {
                overflow_boxes.extend(boxes_for_sp.into_iter().cloned());
            }
        }
    }

    let qc_path = out_dir.join(format!("{}.qc", model_stem));
    let mut has_main_qc = false;

    if !solid_filenames.is_empty() {
        write_qc_file_with_options(
            &qc_path,
            clean_cat,
            &model_stem,
            &solid_filenames,
            phys_filename.as_deref(),
            false,
            false,
            true,
            max_convex_pieces,
            primary_surface_prop,
        )?;
        has_main_qc = true;
    }

    let mut translucent_qc_path = None;
    let mut translucent_model_rel_path = None;
    if !chunk.translucent_tris.is_empty() {
        let trans_prefix = format!("{}_translucent", model_stem);
        let trans_paths = write_visual_smd_parts(
            &chunk.translucent_tris,
            out_dir,
            &trans_prefix,
            chunk.origin_source,
        )?;
        let trans_filenames: Vec<String> = trans_paths
            .iter()
            .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()))
            .collect();
        if !trans_filenames.is_empty() {
            let t_qc = out_dir.join(format!("{}.qc", trans_prefix));
            write_qc_file_with_options(
                &t_qc,
                clean_cat,
                &trans_prefix,
                &trans_filenames,
                None,
                false,
                true,
                false,
                max_convex_pieces,
                "glass",
            )?;
            translucent_qc_path = Some(t_qc);
            translucent_model_rel_path = Some(format!("models/{}/{}.mdl", clean_cat, trans_prefix));
        }
    }

    Ok(CompiledChunkArtifact {
        chunk_name: model_stem.clone(),
        origin_source: chunk.origin_source,
        qc_path: if has_main_qc { qc_path } else { PathBuf::new() },
        model_rel_path: if has_main_qc {
            format!("models/{}/{}.mdl", clean_cat, model_stem)
        } else {
            String::new()
        },
        translucent_qc_path,
        translucent_model_rel_path,
        extra_collision_models,
        extra_collision_boxes,
        overflow_boxes,
        fallback_vmf_brushes: false,
    })
}

fn write_dummy_collision_ref_smd(smd_path: &Path) -> Result<(), String> {
    if let Some(parent) = smd_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }
    // Non-degenerate 1x1x1 HU tetrahedron so VBSP/StudioMDL never warn about flat/bad geometry
    let content = concat!(
        "version 1\nnodes\n0 \"root\" -1\nend\nskeleton\ntime 0\n",
        "0 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000\nend\ntriangles\n",
        "toolsnodraw\n",
        "0 0.000000 0.000000 0.000000 0.000000 0.000000 -1.000000 0.000000 0.000000\n",
        "0 0.000000 1.000000 0.000000 0.000000 0.000000 -1.000000 0.000000 1.000000\n",
        "0 1.000000 0.000000 0.000000 0.000000 0.000000 -1.000000 1.000000 0.000000\n",
        "toolsnodraw\n",
        "0 0.000000 0.000000 0.000000 0.000000 -1.000000 0.000000 0.000000 0.000000\n",
        "0 1.000000 0.000000 0.000000 0.000000 -1.000000 0.000000 1.000000 0.000000\n",
        "0 0.000000 0.000000 1.000000 0.000000 -1.000000 0.000000 0.000000 1.000000\n",
        "toolsnodraw\n",
        "0 0.000000 0.000000 0.000000 -1.000000 0.000000 0.000000 0.000000 0.000000\n",
        "0 0.000000 0.000000 1.000000 -1.000000 0.000000 0.000000 0.000000 1.000000\n",
        "0 0.000000 1.000000 0.000000 -1.000000 0.000000 0.000000 1.000000 0.000000\n",
        "toolsnodraw\n",
        "0 1.000000 0.000000 0.000000 0.577350 0.577350 0.577350 0.000000 0.000000\n",
        "0 0.000000 1.000000 0.000000 0.577350 0.577350 0.577350 1.000000 0.000000\n",
        "0 0.000000 0.000000 1.000000 0.577350 0.577350 0.577350 0.000000 1.000000\n",
        "end\n"
    );
    std::fs::write(smd_path, content)
        .map_err(|e| format!("Failed writing {}: {}", smd_path.display(), e))
}

fn write_collision_only_qc(
    qc_path: &Path,
    category: &str,
    model_name: &str,
    ref_smd: &str,
    phys_smd: &str,
    surface_prop: &str,
    max_convex_pieces: usize,
) -> Result<(), String> {
    if let Some(parent) = qc_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let qc = format!(
        "$modelname \"{}/{}.mdl\"\n$cdmaterials \"tools\"\n$surfaceprop \"{}\"\n$staticprop\n$model \"body\" \"{}\"\n$sequence \"idle\" \"{}\" fps 1\n$collisionmodel \"{}\" {{\n    $concave\n    $automass\n    $maxconvexpieces {}\n}}\n",
        clean_cat, model_name, surface_prop, ref_smd, ref_smd, phys_smd, max_convex_pieces
    );
    std::fs::write(qc_path, qc)
        .map_err(|e| format!("Failed writing {}: {}", qc_path.display(), e))
}
