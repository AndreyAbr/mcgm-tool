#[cfg(test)]
mod suite {
    use crate::anvil::{load_world_region, BlockState, VoxelGrid, WorldCoords};
    use crate::assets::{apply_biome_tint, is_grayscale_image, AssetIndex, FaceDir, ResolvedBlockModel};
    use crate::atlas::{
        build_texture_atlas, export_door_textures, FontRenderer, TexRequestKey,
    };
    use crate::cli::CliArgs;
    use crate::collision::{
        block_local_collision_boxes, build_hollow_shell_for_test, classify_surface_prop,
        greedy_mesh_3d, is_leaves_block, is_passable_block, snow_collision_height, CollisionBox,
        SurfaceProp,
    };
    use crate::compiler::{
        cleanup_staged_files, generate_vmt, pack_gma_v3_native, qc_cache_signature_matches,
        stage_collision_vmts_to_game_dir, vrad_ambient_rgb_for_style, write_addon_metadata,
        write_qc_cache_signature,
    };
    use crate::glua::{generate_map_physics_lua, generate_mcgm_physics_autorun, CROUCHED_WALK_SPEED, JUMP_POWER, STEP_SIZE};
    use crate::gui::{
        build_instance_combo_items, classify_button_kind, compute_card_rects,
        compute_default_client_size, compute_min_client_size, derive_model_name_from_map,
        format_done_summary, format_selection_summary, is_advanced_only_control, light_palette,
        merge_discovered_world, resolve_browse_initial_dir, scale_px,
        should_auto_replace_map_name, tr_text, worlds_for_instance_selection, ControlVisualKind,
        GuiLayoutMode, UiLang, GUI_BOLD_FONT_FACE, GUI_REGULAR_FONT_FACE, ID_BTN_BROWSE_WORLD,
        ID_BTN_RAD_128, ID_BTN_RAD_32, ID_BTN_RAD_64, ID_BTN_START, ID_BTN_STOP,
        ID_CHK_EXPORT_VMF, ID_CHK_IMPORT_GMOD, ID_CHK_MC_PHYSICS, ID_CHK_VMF_BRUSHES, ID_COMBO_DIM,
        ID_EDIT_CAT, ID_EDIT_COORD_X1, ID_EDIT_COORD_X2, ID_EDIT_COORD_Y1, ID_EDIT_COORD_Y2,
        ID_EDIT_COORD_Z1, ID_EDIT_COORD_Z2, ID_EDIT_NAME, ID_EDIT_OUTDIR, ID_EDIT_SCALE,
        ID_EDIT_WORLD, ID_GRP_ADV, ID_LBL_COORDS_MAX, ID_LBL_COORDS_MIN,
    };
    use crate::instances::{
        discover_multimc_family_instances, discover_vanilla_instances, find_instance_by_query,
        parse_ini_value, parse_mmc_pack_metadata, resolve_world_in_instance, scan_custom_dir,
        strip_windows_verbatim_prefix, DiscoveredWorld, LauncherKind, MinecraftInstance,
    };
    use crate::mesher::{should_cull_face, InteractableDoor, MeshTri};
    use crate::skybox::generate_minecraft_skybox;
    use crate::smd_writer::{emit_chunk_smds_and_qcs, write_physics_smd, write_qc_file, write_visual_smd_parts, CompiledChunkArtifact, MAX_VERTS_PER_SMD};
    use crate::vmf_builder::{build_vmf, count_vmf_entities, vmf_contains_kv, VmfBuildParams};
    use clap::Parser;
    use flate2::{write::ZlibEncoder, Compression};
    use image::{ImageBuffer, ImageFormat, Rgba};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::io::{Cursor, Write};

    fn make_state(name: &str, pairs: &[(&str, &str)]) -> BlockState {
        let mut props = BTreeMap::new();
        for (k, v) in pairs {
            props.insert((*k).to_string(), (*v).to_string());
        }
        BlockState::new(name.to_string(), props)
    }

    fn encode_png_bytes(img: &image::RgbaImage) -> Vec<u8> {
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .expect("png encode");
        buf
    }

    #[test]
    fn test_atlas_crop_biome_tint_and_sign_text() {
        let mut assets = AssetIndex::empty();

        // 16x64 animated grayscale water strip
        let anim_img: image::RgbaImage = ImageBuffer::from_pixel(16, 64, Rgba([180, 180, 180, 255]));
        assets.insert_entry("assets/minecraft/textures/block/water_still.png", encode_png_bytes(&anim_img));
        assets.insert_entry(
            "assets/minecraft/textures/block/water_still.png.mcmeta",
            br#"{"animation":{"frametime":2}}"#.to_vec(),
        );

        let loaded_water = assets.load_texture_image("minecraft:block/water_still").unwrap();
        assert_eq!(loaded_water.dimensions(), (16, 16), "Animated strip must crop to 16x16 frame 0");

        // Grayscale leaves vs already-colored pink cherry leaves
        let gray_leaf: image::RgbaImage = ImageBuffer::from_pixel(16, 16, Rgba([200, 200, 200, 255]));
        let cherry_leaf: image::RgbaImage = ImageBuffer::from_pixel(16, 16, Rgba([245, 165, 205, 255]));
        assert!(is_grayscale_image(&gray_leaf));
        assert!(!is_grayscale_image(&cherry_leaf));
        let tinted = apply_biome_tint(&gray_leaf, [89, 174, 48]);
        assert!(tinted.get_pixel(0, 0)[1] > tinted.get_pixel(0, 0)[0]);

        // Atlas packing & UV clamping
        let mut reqs = HashSet::new();
        reqs.insert(TexRequestKey {
            tex_ref: "minecraft:block/water_still".to_string(),
            tint_rgb: Some([60, 116, 228]),
        });
        let atlas = build_texture_atlas(&reqs, &assets, 8);
        assert_eq!(atlas.pad, 8);
        let uvr = atlas.get_or_default(reqs.iter().next().unwrap());
        let [u0, v0] = uvr.map_uv(0.0, 0.0);
        let [u1, v1] = uvr.map_uv(1.0, 1.0);
        assert!(u0 > 0.0 && u1 < 1.0 && v1 < v0 && v0 < 1.0 && v1 > 0.0);

        // Sign text rendering without external font JAR
        let font = FontRenderer::from_assets(&assets);
        let sign_panel = font.render_sign_panel(
            &[
                "Привет Мир".to_string(),
                "Тест 123".to_string(),
                String::new(),
                String::new(),
            ],
            "black",
            [162, 130, 78],
        );
        assert_eq!(sign_panel.dimensions(), (256, 128));
        let dark_ink_pixels = sign_panel
            .pixels()
            .filter(|p| p[0] < 40 && p[1] < 40 && p[2] < 40)
            .count();
        assert!(dark_ink_pixels > 80, "Sign text must render ink pixels onto sign");
    }

    #[test]
    fn test_face_culling_non_full_blocks_and_leaves() {
        let stone = make_state("minecraft:stone", &[]);
        let dirt_path = make_state("minecraft:dirt_path", &[]);
        let farmland = make_state("minecraft:farmland", &[]);
        let stairs = make_state("minecraft:oak_stairs", &[("facing", "north")]);
        let slab_bot = make_state("minecraft:oak_slab", &[("type", "bottom")]);
        let slab_dbl = make_state("minecraft:oak_slab", &[("type", "double")]);
        let fence = make_state("minecraft:oak_fence", &[]);
        let leaves = make_state("minecraft:oak_leaves", &[]);

        assert!(should_cull_face(&stone, &stone, FaceDir::North));
        assert!(should_cull_face(&stone, &slab_dbl, FaceDir::North));
        assert!(!should_cull_face(&stone, &dirt_path, FaceDir::South));
        assert!(!should_cull_face(&stone, &farmland, FaceDir::South));
        assert!(!should_cull_face(&stone, &stairs, FaceDir::East));
        assert!(!should_cull_face(&stone, &slab_bot, FaceDir::West));
        assert!(!should_cull_face(&stone, &fence, FaceDir::Up));
        assert!(!should_cull_face(&stone, &leaves, FaceDir::Up));
    }

    #[test]
    fn test_collision_leaves_snow_hollow_and_greedy() {
        let leaves = make_state("minecraft:oak_leaves", &[]);
        let leaf_litter = make_state("minecraft:leaf_litter", &[]);
        let snow1 = make_state("minecraft:snow", &[("layers", "1")]);
        let snow4 = make_state("minecraft:snow", &[("layers", "4")]);

        assert!(is_leaves_block(&leaves));
        assert!(!block_local_collision_boxes(&leaves).is_empty());
        assert!(is_passable_block(&leaf_litter));
        assert!(is_passable_block(&snow1));
        assert_eq!(snow_collision_height(1), None);
        assert_eq!(snow_collision_height(4), Some(0.375));
        assert_eq!(block_local_collision_boxes(&snow4)[0].0[4], 0.375);

        // Exposed shell on 3x3x3 solid stone cube
        let stone = make_state("minecraft:stone", &[]);
        let air = make_state("minecraft:air", &[]);
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 0, min_z: 0, max_x: 2, max_y: 2, max_z: 2 },
            palette: vec![air, stone],
            air_id: 0,
            blocks: vec![1; 27],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 27],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let shell = build_hollow_shell_for_test(&grid);
        assert_eq!(shell.len(), 26, "3x3x3 solid block should remove 1 interior voxel");
        assert!(!shell.contains(&(1, 1, 1)));

        // 3D Greedy Meshing 4x1x4 block -> 1 box
        let mut mask = vec![true; 16];
        let boxes = greedy_mesh_3d(&mut mask, 4, 1, 4);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0], [0, 0, 0, 4, 1, 4]);
    }

    #[test]
    fn test_smd_split_and_qc() {
        let tmp = std::env::temp_dir().join("mcgm_test_smd");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let tri = MeshTri {
            material: "terrain_opaque".into(),
            pos: [[10.0, 20.0, 0.0], [20.0, 20.0, 0.0], [20.0, 30.0, 0.0]],
            norm: [[0.0, 0.0, 1.0]; 3],
            uv: [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9]],
        };
        let tris = vec![tri; 20_000];
        let parts = write_visual_smd_parts(&tris, &tmp, "chunk_0_0", [10.0, 20.0, 0.0]).unwrap();
        assert_eq!(parts.len(), 3, "20,000 tris = 60,000 verts must split into 3 parts <= 28,000 verts each");
        const { assert!(MAX_VERTS_PER_SMD == 28_000); }

        let part0 = std::fs::read_to_string(&parts[0]).unwrap();
        assert!(part0.starts_with("version 1\nnodes\n"));
        assert!(part0.contains("triangles\nterrain_opaque"));

        let cbox = CollisionBox {
            box_coords: [0.0, 0.0, 0.0, 39.37, 39.37, 39.37],
            block_class: "full".into(),
            is_walkable: true,
            is_opaque_roof: true,
            surface_prop: SurfaceProp::Rock,
        };
        let phys_path = tmp.join("chunk_0_0_phys.smd");
        let overflow = write_physics_smd(&[cbox], &phys_path, [0.0, 0.0, 0.0], 2048, 0.015).unwrap();
        assert!(overflow.is_empty());
        assert!(phys_path.is_file());

        let qc_path = tmp.join("chunk_0_0.qc");
        write_qc_file(
            &qc_path,
            "custom_mc",
            "chunk_0_0",
            &["chunk_0_0_part_0.smd".to_string()],
            Some("chunk_0_0_phys.smd"),
            true,
            true,
            2048,
        )
        .unwrap();
        let qc = std::fs::read_to_string(&qc_path).unwrap();
        assert!(qc.contains("$staticprop"));
        assert!(qc.contains("$mostlyopaque"));
        assert!(qc.contains("$collisionmodel \"chunk_0_0_phys.smd\""));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_doors_vmf_skybox_and_lightmappedgeneric() {
        let tmp = std::env::temp_dir().join("mcgm_test_vmf");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 16x32 door PNG export & LightmappedGeneric VMT
        let mut assets = AssetIndex::empty();
        let top = ImageBuffer::from_pixel(16, 16, Rgba([160, 120, 70, 255]));
        let bot = ImageBuffer::from_pixel(16, 16, Rgba([140, 100, 60, 255]));
        assets.insert_entry("assets/minecraft/textures/block/oak_door_top.png", encode_png_bytes(&top));
        assets.insert_entry("assets/minecraft/textures/block/oak_door_bottom.png", encode_png_bytes(&bot));
        let mut door_set = HashSet::new();
        door_set.insert("minecraft:oak_door".to_string());
        let door_pngs = export_door_textures(&door_set, &assets, &tmp);
        let oak_png = image::open(&door_pngs["door_oak_door"]).unwrap();
        assert_eq!((oak_png.width(), oak_png.height()), (16, 32));

        let vmt_path = tmp.join("door_oak_door.vmt");
        generate_vmt(&vmt_path, "LightmappedGeneric", "custom_mc/door_oak_door", "wood", true, false, false, false, false).unwrap();
        let vmt = std::fs::read_to_string(&vmt_path).unwrap();
        assert!(vmt.contains("LightmappedGeneric"));

        // Skybox 6 faces
        let sky_faces = generate_minecraft_skybox("minecraft", &tmp.join("sky")).unwrap();
        assert_eq!(sky_faces.len(), 6);
        for (_, p) in &sky_faces {
            let im = image::open(p).unwrap();
            assert_eq!((im.width(), im.height()), (512, 512));
        }

        // VMF Build: func_door_rotating speed 9000, func_detail, env_fog_controller farz >= 6000
        let vmf_path = tmp.join("test.vmf");
        let door = InteractableDoor {
            mc_pos: [0, 64, 0],
            box_coords: [0.0, 0.0, 0.0, 39.37, 7.38, 78.74],
            origin: [3.69, 3.69, 39.37],
            facing: "north".to_string(),
            hinge: "left".to_string(),
            open: false,
            spawnflags: 306,
            mat_name: "door_oak_door".to_string(),
            short_type: "oak_door".to_string(),
        };
        let col = CollisionBox {
            box_coords: [-39.37, -39.37, 0.0, 39.37, 39.37, 39.37],
            block_class: "full".into(),
            is_walkable: true,
            is_opaque_roof: true,
            surface_prop: SurfaceProp::Rock,
        };
        let art = CompiledChunkArtifact {
            chunk_name: "chunk_0_0".to_string(),
            origin_source: [0.0, 0.0, 0.0],
            qc_path: tmp.join("chunk_0_0.qc"),
            model_rel_path: "models/custom_mc/chunk_0_0.mdl".to_string(),
            translucent_qc_path: None,
            translucent_model_rel_path: None,
            extra_collision_models: Vec::new(),
            extra_collision_boxes: Vec::new(),
            overflow_boxes: vec![col.clone()],
            fallback_vmf_brushes: false,
        };
        build_vmf(&VmfBuildParams {
            output_vmf: &vmf_path,
            category: "custom_mc",
            bounds_min: [-200.0, -200.0, 0.0],
            bounds_max: [200.0, 200.0, 200.0],
            chunks: &[art],
            all_solid_boxes: &[col],
            ladder_boxes: &[],
            water_pools: &[],
            hazard_boxes: &[],
            core_occluders: &[],
            doors: &[door],
            buttons: &[],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 48.0]],
            lighting_style: "minecraft",
            hq_light: false,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();

        let vmf_text = std::fs::read_to_string(&vmf_path).unwrap();
        assert_eq!(count_vmf_entities(&vmf_text, "func_door_rotating"), 1);
        assert!(vmf_contains_kv(&vmf_text, "speed", "350"));
        assert!(vmf_contains_kv(&vmf_text, "spawnflags", "306"));
        assert!(vmf_text.contains("2.460625"));
        assert!(count_vmf_entities(&vmf_text, "func_detail") >= 1);
        assert!(vmf_contains_kv(&vmf_text, "farz", "6000"));
        assert!(vmf_contains_kv(&vmf_text, "skyname", "mc_sky_minecraft"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_glua_physics_constants_and_cli_gui_ru() {
        assert!((JUMP_POWER - 329.36).abs() < 1e-2);
        assert!((CROUCHED_WALK_SPEED - (1.30 / 4.317)).abs() < 1e-6);
        assert!((STEP_SIZE - (0.60 * 39.37)).abs() < 1e-4);

        let lua = generate_mcgm_physics_autorun();
        assert!(lua.contains("ply:SetGravity(desiredG / svG)"));
        assert!(lua.contains("CrouchedWalkSpeed = 1.30 / 4.317"));
        assert!(lua.contains("StepSize = 0.60 * DefaultScale"));
        assert!(lua.contains("PlayerHalfWidth = 0.29"));
        assert!(lua.contains("PlayerHeight = 1.78"));
        assert!(lua.contains("TryAutoJump"));

        let map_lua = generate_map_physics_lua("gm_mc_castle_mega3", 39.37, true, &[]);
        assert!(map_lua.contains("gm_mc_castle_mega3"));

        // CLI compatibility including negative coords and --dimension
        let cli = CliArgs::try_parse_from([
            "mcgm",
            "--world",
            r"H:\Games\PrismLauncher\instances\mcc\minecraft\saves\worlds",
            "--coords",
            "9120",
            "65",
            "-6060",
            "9370",
            "130",
            "-5810",
            "--dimension",
            "overworld",
            "--name",
            "gm_mc_castle_mega",
            "--category",
            "custom_mc",
            "--map",
            "gm_mc_castle_mega3",
            "--import-to-gmod",
            "--export-vmf",
            "--lighting-style",
            "minecraft",
            "--fast-vrad",
        ])
        .unwrap();
        let c = cli.parsed_coords().unwrap();
        assert_eq!((c.min_x, c.min_y, c.min_z, c.max_x, c.max_y, c.max_z), (9120, 65, -6060, 9370, 130, -5810));
        assert!(cli.effective_import_to_gmod());
        assert!(cli.export_vmf);

        // Natural sentence case Russian GUI strings
        assert_eq!(tr_text(UiLang::Ru, "chk_import"), "Импортировать напрямую в Garry's Mod");
        assert_eq!(tr_text(UiLang::Ru, "chk_vmf"), "Сохранить исходник карты (.vmf) для Hammer");
        assert_eq!(tr_text(UiLang::Ru, "grp_meta"), "Настройки модели и карты");
    }

    #[test]
    fn test_anvil_nbt_and_sign_extraction() {
        fn push_nbt_str(buf: &mut Vec<u8>, s: &str) {
            let b = s.as_bytes();
            buf.extend_from_slice(&(b.len() as u16).to_be_bytes());
            buf.extend_from_slice(b);
        }
        fn push_tag_header(buf: &mut Vec<u8>, tag: u8, name: &str) {
            buf.push(tag);
            push_nbt_str(buf, name);
        }
        fn push_int(buf: &mut Vec<u8>, name: &str, v: i32) {
            push_tag_header(buf, 3, name);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        fn push_byte(buf: &mut Vec<u8>, name: &str, v: i8) {
            push_tag_header(buf, 1, name);
            buf.push(v as u8);
        }
        fn push_str_tag(buf: &mut Vec<u8>, name: &str, val: &str) {
            push_tag_header(buf, 8, name);
            push_nbt_str(buf, val);
        }

        // Construct minimal valid 1.20.1 chunk NBT payload
        let mut nbt = Vec::new();
        push_tag_header(&mut nbt, 10, "");
        push_int(&mut nbt, "DataVersion", 3465);
        push_int(&mut nbt, "xPos", 0);
        push_int(&mut nbt, "zPos", 0);

        // sections: List<Compound> (1 section at Y=4 -> Y 64..79)
        push_tag_header(&mut nbt, 9, "sections");
        nbt.push(10); // element type: Compound
        nbt.extend_from_slice(&1i32.to_be_bytes());
        push_byte(&mut nbt, "Y", 4);

        // block_states: Compound
        push_tag_header(&mut nbt, 10, "block_states");
        push_tag_header(&mut nbt, 9, "palette");
        nbt.push(10);
        nbt.extend_from_slice(&2i32.to_be_bytes());
        // palette[0] = minecraft:stone
        push_str_tag(&mut nbt, "Name", "minecraft:stone");
        nbt.push(0);
        // palette[1] = minecraft:oak_sign[facing=south]
        push_str_tag(&mut nbt, "Name", "minecraft:oak_sign");
        push_tag_header(&mut nbt, 10, "Properties");
        push_str_tag(&mut nbt, "facing", "south");
        nbt.push(0);
        nbt.push(0);

        // Packed data: 4096 blocks, 4 bits/entry -> 256 longs; local (x=2, y=1, z=3) => idx 306 -> palette[1]
        let mut longs = vec![0i64; 256];
        let vox_idx = 256usize + 3 * 16 + 2; // 306
        longs[vox_idx / 16] |= 1i64 << ((vox_idx % 16) * 4);
        push_tag_header(&mut nbt, 12, "data");
        nbt.extend_from_slice(&(longs.len() as i32).to_be_bytes());
        for l in longs {
            nbt.extend_from_slice(&l.to_be_bytes());
        }
        nbt.push(0); // end block_states

        // biomes: Compound with single biome minecraft:cherry_grove
        push_tag_header(&mut nbt, 10, "biomes");
        push_tag_header(&mut nbt, 9, "palette");
        nbt.push(8); // String list
        nbt.extend_from_slice(&1i32.to_be_bytes());
        push_nbt_str(&mut nbt, "minecraft:cherry_grove");
        nbt.push(0); // end biomes

        nbt.push(0); // end section 0

        // block_entities: List<Compound> containing 1 sign at (2, 65, 3)
        push_tag_header(&mut nbt, 9, "block_entities");
        nbt.push(10);
        nbt.extend_from_slice(&1i32.to_be_bytes());
        push_str_tag(&mut nbt, "id", "minecraft:sign");
        push_int(&mut nbt, "x", 2);
        push_int(&mut nbt, "y", 65);
        push_int(&mut nbt, "z", 3);
        push_tag_header(&mut nbt, 10, "front_text");
        push_str_tag(&mut nbt, "color", "yellow");
        push_tag_header(&mut nbt, 9, "messages");
        nbt.push(8);
        nbt.extend_from_slice(&4i32.to_be_bytes());
        push_nbt_str(&mut nbt, r#"{"text":"Привет Замок"}"#);
        push_nbt_str(&mut nbt, r#"{"text":"Линия 2"}"#);
        push_nbt_str(&mut nbt, "");
        push_nbt_str(&mut nbt, "");
        nbt.push(0); // end front_text
        nbt.push(0); // end sign block entity

        nbt.push(0); // end root

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&nbt).unwrap();
        let compressed = enc.finish().unwrap();

        let payload_len = (compressed.len() + 1) as u32;
        let total_chunk_bytes = 4 + 1 + compressed.len();
        let sector_count = total_chunk_bytes.div_ceil(4096) as u8;
        let mut mca = vec![0u8; 8192 + (sector_count as usize) * 4096];
        // Chunk (0, 0) starts at sector 2
        mca[0] = 0;
        mca[1] = 0;
        mca[2] = 2;
        mca[3] = sector_count;
        mca[8192..8196].copy_from_slice(&payload_len.to_be_bytes());
        mca[8196] = 2; // zlib
        mca[8197..8197 + compressed.len()].copy_from_slice(&compressed);

        let world_dir = std::env::temp_dir().join(format!("mcgm_test_anvil_{}", std::process::id()));
        let region_dir = world_dir.join("region");
        std::fs::create_dir_all(&region_dir).unwrap();
        std::fs::write(region_dir.join("r.0.0.mca"), &mca).unwrap();

        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 3,
            max_y: 66,
            max_z: 3,
        };
        let grid = load_world_region(&world_dir, "overworld", coords, 39.37).expect("Anvil region load");
        let _ = std::fs::remove_dir_all(&world_dir);

        assert_eq!(grid.get_block(0, 64, 0).name, "minecraft:stone");
        let sign_bs = grid.get_block(2, 65, 3);
        assert_eq!(sign_bs.name, "minecraft:oak_sign");
        assert_eq!(sign_bs.prop("facing"), Some("south"));
        assert_eq!(grid.get_biome(2, 65, 3), "minecraft:cherry_grove");
        let sign = grid.signs.get(&(2, 65, 3)).expect("Sign must be extracted");
        assert_eq!(sign.front_color, "yellow");
        assert_eq!(sign.front_lines[0], "Привет Замок");
        assert_eq!(sign.front_lines[1], "Линия 2");
    }

    #[test]
    fn test_create_obj_and_blockstate_variant() {
        let mut assets = AssetIndex::empty();

        assets.insert_entry(
            "assets/create/blockstates/cogwheel.json",
            br#"{"variants":{"axis=y":{"model":"create:block/cogwheel_shaftless","y":90},"axis=x":{"model":"create:block/cogwheel_shaftless","x":90}}}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/create/models/block/cogwheel_shaftless.json",
            br#"{"loader":"forge:obj","model":"create:models/block/cogwheel.obj"}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/create/models/block/cogwheel.mtl",
            b"newmtl brass\nmap_Kd create:block/brass_block\n".to_vec(),
        );
        assets.insert_entry(
            "assets/create/models/block/cogwheel.obj",
            b"mtllib cogwheel.mtl\nusemtl brass\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\n".to_vec(),
        );

        let cog_y = make_state("create:cogwheel", &[("axis", "y")]);
        match assets.resolve_block_model(&cog_y) {
            ResolvedBlockModel::Obj(tris) => {
                assert_eq!(tris.len(), 2, "Quad OBJ face must triangulate into 2 triangles");
                assert_eq!(tris[0].texture, "create:block/brass_block");
            }
            other => panic!("Expected Obj model, got {:?}", other),
        }

        // JSON parent chain + texture variable inheritance
        assets.insert_entry(
            "assets/mcwbridges/blockstates/custom_post.json",
            br#"{"variants":{"":{"model":"mcwbridges:block/custom_post"}}}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/mcwbridges/models/block/base_post.json",
            br##"{"elements":[{"from":[4,0,4],"to":[12,16,12],"faces":{"north":{"texture":"#side"},"up":{"texture":"#top"}}}]}"##.to_vec(),
        );
        assets.insert_entry(
            "assets/mcwbridges/models/block/custom_post.json",
            br#"{"parent":"mcwbridges:block/base_post","textures":{"side":"minecraft:block/oak_planks","top":"minecraft:block/oak_log_top"}}"#.to_vec(),
        );
        let post_state = make_state("mcwbridges:custom_post", &[]);
        match assets.resolve_block_model(&post_state) {
            ResolvedBlockModel::Elements(els) => {
                assert_eq!(els.len(), 1);
                assert_eq!(els[0].faces.get(&FaceDir::North).unwrap().texture, "minecraft:block/oak_planks");
                assert_eq!(els[0].faces.get(&FaceDir::Up).unwrap().texture, "minecraft:block/oak_log_top");
            }
            other => panic!("Expected Elements model, got {:?}", other),
        }
    }

    #[test]
    fn test_slab_merge_priority_and_multi_chunk_overflow() {
        use crate::collision::build_world_collision;
        // 4x2x4 region with 4 bottom slabs that must merge into 1 CollisionBox
        let palette = vec![
            make_state("minecraft:air", &[]),
            make_state("minecraft:stone_brick_slab", &[("type", "bottom")]),
        ];
        let mut blocks = vec![0u16; 4 * 2 * 4];
        for z in 0..4usize {
            for x in 0..4usize {
                blocks[z * 4 + x] = 1;
            }
        }
        let grid = VoxelGrid {
            coords: WorldCoords {
                min_x: 0,
                min_y: 64,
                min_z: 0,
                max_x: 3,
                max_y: 65,
                max_z: 3,
            },
            palette,
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0u16; 4 * 2 * 4],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision(&grid, [0.0, 64.0, 0.0], 41.0);
        let ch_boxes = col.per_chunk.get(&(0, 0)).expect("chunk 0,0 boxes");
        assert_eq!(ch_boxes.len(), 1, "Contiguous 4x4 bottom slabs must greedy-merge into 1 box");
        assert_eq!(ch_boxes[0].block_class, "slab");

        // Priority sort: walkable floor must outrank canopy leaves when capped
        let temp_smd = std::env::temp_dir().join(format!("mcgm_test_prio_{}.smd", std::process::id()));
        let boxes = vec![
            CollisionBox {
                box_coords: [0.0, 0.0, 500.0, 40.0, 40.0, 540.0],
                block_class: "leaves".into(),
                is_walkable: true,
                is_opaque_roof: false,
                surface_prop: SurfaceProp::Grass,
            },
            CollisionBox {
                box_coords: [0.0, 0.0, 0.0, 40.0, 40.0, 40.0],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            },
        ];
        let overflow = write_physics_smd(&boxes, &temp_smd, [0.0, 0.0, 0.0], 1, 0.015).unwrap();
        let _ = std::fs::remove_file(&temp_smd);
        assert_eq!(overflow.len(), 1);
        assert_eq!(overflow[0].block_class, "leaves", "Leaves must overflow before walkable full floors");
    }

    #[test]
    fn test_generate_map_thumbnail_outputs_icon_and_thumb() {
        use crate::compiler::generate_map_thumbnail;
        let base = std::env::temp_dir().join(format!("mcgm_test_thumb_{}", std::process::id()));
        let addon = base.join("addon");
        let game = base.join("game");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::create_dir_all(&game).unwrap();

        generate_map_thumbnail("gm_mc_castle_mega3", &addon, Some(&game), "minecraft").unwrap();

        let thumb_p = addon.join("maps").join("thumb").join("gm_mc_castle_mega3.png");
        let icon_p = addon.join("maps").join("gm_mc_castle_mega3.png");
        let g_thumb = game.join("maps").join("thumb").join("gm_mc_castle_mega3.png");
        let g_icon = game.join("maps").join("gm_mc_castle_mega3.png");

        assert!(thumb_p.is_file() && g_thumb.is_file());
        assert!(icon_p.is_file() && g_icon.is_file());

        let img512 = image::open(&thumb_p).unwrap().to_rgba8();
        let img256 = image::open(&icon_p).unwrap().to_rgba8();
        assert_eq!(img512.dimensions(), (512, 512));
        assert_eq!(img256.dimensions(), (256, 256));

        // Sky pixel at (100, 20) must be bright sky blue; grass at (250, 340) must be green
        let sky_px = img512.get_pixel(100, 20).0;
        assert!(sky_px[2] == 255 && sky_px[0] >= 120 && sky_px[1] >= 165);
        let grass_px = img512.get_pixel(250, 340).0;
        assert_eq!(grass_px, [86, 148, 48, 255]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_wall_collision_leaves_retention_culling_vmt_and_nodegraph() {
        use crate::atlas::{classify_texture_bucket, RenderBucket};
        use crate::collision::build_world_collision;
        use crate::glua::install_mc_physics_files;

        // 1. Stone wall arm collision (north=low, east=tall, south/west=none)
        let wall = make_state(
            "minecraft:cobblestone_wall",
            &[("north", "low"), ("east", "tall"), ("south", "none"), ("west", "none")],
        );
        let wall_boxes = block_local_collision_boxes(&wall);
        assert_eq!(wall_boxes.len(), 3, "Wall center post + 2 connected arms");
        assert!(wall_boxes.iter().all(|(b, cls, _)| *cls == "wall" && (b[4] - 1.5).abs() < 1e-6));
        assert!(wall_boxes.iter().any(|(b, _, _)| (b[2] - 0.0).abs() < 1e-6 && (b[5] - 0.5).abs() < 1e-6));
        assert!(wall_boxes.iter().any(|(b, _, _)| (b[0] - 0.5).abs() < 1e-6 && (b[3] - 1.0).abs() < 1e-6));

        // 2. Leaves collision retention (flowering_azalea_leaves solid, 3x3x3 canopy not hollowed out)
        let fl_leaves = make_state("minecraft:flowering_azalea_leaves", &[]);
        assert!(is_leaves_block(&fl_leaves));
        assert!(!is_passable_block(&fl_leaves));

        let air = make_state("minecraft:air", &[]);
        let oak_leaves = make_state("minecraft:oak_leaves", &[]);
        let grid_leaves = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 2, max_y: 66, max_z: 2 },
            palette: vec![air, oak_leaves],
            air_id: 0,
            blocks: vec![1; 27],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 27],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col_leaves = build_world_collision(&grid_leaves, [0.0, 64.0, 0.0], 41.0);
        let leaves_boxes = col_leaves.per_chunk.get(&(0, 0)).expect("leaves chunk boxes");
        assert_eq!(leaves_boxes.len(), 1, "3x3x3 leaves canopy should greedy-merge without hollowing");
        assert_eq!(leaves_boxes[0].block_class, "leaves");

        // 3. grass_block top face NOT culled under snow[layers=1] or flowers
        let grass_block = make_state("minecraft:grass_block", &[]);
        let snow1 = make_state("minecraft:snow", &[("layers", "1")]);
        let dandelion = make_state("minecraft:dandelion", &[]);
        let poppy = make_state("minecraft:poppy", &[]);
        let short_grass = make_state("minecraft:short_grass", &[]);
        assert!(!should_cull_face(&grass_block, &snow1, FaceDir::Up));
        assert!(!should_cull_face(&grass_block, &dandelion, FaceDir::Up));
        assert!(!should_cull_face(&grass_block, &poppy, FaceDir::Up));
        assert!(!should_cull_face(&grass_block, &short_grass, FaceDir::Up));

        // 4. VMT $alphatest vs $translucent + texture bucket classification
        let tmp = std::env::temp_dir().join(format!("mcgm_test_vmt_ng_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let cutout_vmt = tmp.join("cutout.vmt");
        let trans_vmt = tmp.join("translucent.vmt");
        generate_vmt(&cutout_vmt, "VertexLitGeneric", "models/mc/atlas", "foliage", true, false, true, false, false).unwrap();
        generate_vmt(&trans_vmt, "VertexLitGeneric", "models/mc/atlas", "glass", false, true, false, false, false).unwrap();
        let cutout_txt = std::fs::read_to_string(&cutout_vmt).unwrap();
        let trans_txt = std::fs::read_to_string(&trans_vmt).unwrap();
        assert!(cutout_txt.contains("\"$alphatest\" \"1\""));
        assert!(cutout_txt.contains("\"$alphatestreference\" \"0.5\""));
        assert!(cutout_txt.contains("\"$nocull\" \"1\""));
        assert!(!cutout_txt.contains("$translucent"));
        assert!(trans_txt.contains("\"$translucent\" \"1\""));
        assert!(!trans_txt.contains("$alphatest"));
        assert!(!trans_txt.contains("$nocull"));

        let opaque_img = ImageBuffer::from_pixel(4, 4, Rgba([200, 220, 255, 255]));
        assert_eq!(classify_texture_bucket("minecraft:block/packed_ice", &opaque_img), RenderBucket::TerrainOpaque);
        assert_eq!(classify_texture_bucket("minecraft:block/dandelion", &opaque_img), RenderBucket::TerrainCutout);
        assert_eq!(classify_texture_bucket("minecraft:block/water_still", &opaque_img), RenderBucket::TerrainTranslucent);

        // 5. No stale/broken node graph trigger
        let autorun = generate_mcgm_physics_autorun();
        assert!(autorun.contains("ai_norebuildgraph"));
        let game_root = tmp.join("garrysmod");
        let lua_dir = game_root.join("lua");
        let graphs_dir = game_root.join("maps").join("graphs");
        std::fs::create_dir_all(&graphs_dir).unwrap();
        let stale_ain = graphs_dir.join("gm_mc_test_ng.ain");
        std::fs::write(&stale_ain, b"stale").unwrap();
        assert!(stale_ain.is_file());
        install_mc_physics_files("gm_mc_test_ng", &[lua_dir.as_path()], 39.37, true, &[]).unwrap();
        assert!(!stale_ain.is_file(), "Stale .ain file must be removed on install");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_glua_map_name_injection_and_path_safety() {
        use crate::glua::install_mc_physics_files;

        // Hostile map names must not break out of the generated Lua string literal.
        let evil = "gm_\"..os.exit()..\"";
        let lua = generate_map_physics_lua(evil, 39.37, true, &[]);
        assert!(
            lua.contains("MCGM_PHYSICS.Maps[\"gm_\\\"..os.exit()..\\\"\"]"),
            "quotes in map names must be escaped in the Lua table key"
        );
        assert!(
            lua.contains("MCGM_RegisterMap(\"gm_\\\"..os.exit()..\\\"\", mapData)"),
            "quotes in map names must be escaped in MCGM_RegisterMap"
        );
        // Normal names pass through unchanged
        let clean = generate_map_physics_lua("gm_mc_test", 39.37, true, &[]);
        assert!(clean.contains("MCGM_PHYSICS.Maps[\"gm_mc_test\"]"));

        // Path traversal names must be sanitized into a safe file stem (no .., no separators)
        let tmp = std::env::temp_dir().join(format!("mcgm_glua_evil_{:x}", std::process::id()));
        let lua_dir = tmp.join("lua");
        install_mc_physics_files("..\\..\\evil", &[lua_dir.as_path()], 39.37, true, &[]).unwrap();
        let cfg_dir = lua_dir.join("mcgm_physics");
        let written: Vec<String> = std::fs::read_dir(&cfg_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(written.len(), 1, "exactly one map config file");
        let fname = &written[0];
        assert!(fname.ends_with(".lua"));
        assert!(
            !fname.contains("..") && !fname.contains('\\') && !fname.contains('/'),
            "sanitized filename must not contain traversal or separator characters: {}",
            fname
        );
        // The generated content itself must escape backslashes inside the Lua key
        // (so the table key is a well-formed Lua string, never a raw path fragment)
        let content = std::fs::read_to_string(cfg_dir.join(fname)).unwrap();
        assert!(
            content.contains("MCGM_PHYSICS.Maps[\"..\\\\..\\\\evil\"]"),
            "backslashes in the map name must be Lua-escaped, got: {}",
            content.lines().last().unwrap_or("")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cli_defaults_short_flags_and_derived_map() {
        // Default --import-to-gmod and --mc-physics should be true when --world is given
        let cli = CliArgs::try_parse_from([
            "mcgm",
            "-w",
            r"C:\saves\my_world",
            "-n",
            "castle_build",
            "-c",
            "custom_mc",
        ])
        .unwrap();
        assert!(cli.effective_import_to_gmod());
        assert!(cli.effective_mc_physics());
        assert_eq!(cli.effective_name(), "castle_build");
        assert_eq!(cli.effective_map(), "gm_castle_build");

        // --no-mc-physics disables mc_physics
        let cli_no_phys = CliArgs::try_parse_from([
            "mcgm",
            "-w",
            r"C:\saves\my_world",
            "-n",
            "castle_build",
            "--no-mc-physics",
        ])
        .unwrap();
        assert!(!cli_no_phys.effective_mc_physics());
    }

    #[test]
    fn test_anvil_flowing_water_bounds_clamp_and_dimension_errors() {
        use crate::anvil::{check_and_clamp_bounds, resolve_dimension_region_dir};

        // 1. flowing_water must be flagged as water
        let fw = make_state("minecraft:flowing_water", &[("level", "3")]);
        assert!(fw.is_water_tag, "flowing_water must set is_water_tag = true");
        let w = make_state("minecraft:water", &[("level", "0")]);
        assert!(w.is_water_tag);

        // 2. Source Engine bounds clamping ([-16384..16384] HU at scale 39.37 -> ~416 blocks max span)
        let huge = WorldCoords {
            min_x: -500,
            min_y: -64,
            min_z: -500,
            max_x: 500,
            max_y: 320,
            max_z: 500,
        };
        let clamped = check_and_clamp_bounds(huge, 39.37);
        assert!(
            (clamped.size_x() as f64) * 39.37 <= 32768.0 + 39.37,
            "Clamped X span must fit within Source Engine bounds"
        );
        assert!(
            (clamped.size_z() as f64) * 39.37 <= 32768.0 + 39.37,
            "Clamped Z span must fit within Source Engine bounds"
        );

        // 3. resolve_dimension_region_dir must return Err instead of silent Overworld fallback
        let tmp = std::env::temp_dir().join(format!("mcgm_test_dim_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("region")).unwrap();
        assert!(resolve_dimension_region_dir(&tmp, "the_nether").is_err());
        assert!(resolve_dimension_region_dir(&tmp, "the_end").is_err());
        std::fs::create_dir_all(tmp.join("DIM-1").join("region")).unwrap();
        assert!(resolve_dimension_region_dir(&tmp, "dim-1").is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_assets_vanilla_jar_validation_uvlock_and_fluid_flow() {
        use crate::assets::is_valid_vanilla_jar;
        use zip::write::FileOptions;
        use zip::ZipWriter;

        let tmp = std::env::temp_dir().join(format!("mcgm_test_jar_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Loader JAR without stone.png -> invalid
        let loader_jar = tmp.join("fabric-loader-0.15.0-1.20.1.jar");
        {
            let f = std::fs::File::create(&loader_jar).unwrap();
            let mut zw = ZipWriter::new(f);
            zw.start_file::<_, ()>("fabric.mod.json", FileOptions::default()).unwrap();
            zw.write_all(b"{}").unwrap();
            zw.finish().unwrap();
        }
        assert!(!is_valid_vanilla_jar(&loader_jar));

        // Vanilla JAR with assets/minecraft/textures/block/stone.png -> valid
        let vanilla_jar = tmp.join("1.20.1.jar");
        {
            let f = std::fs::File::create(&vanilla_jar).unwrap();
            let mut zw = ZipWriter::new(f);
            zw.start_file::<_, ()>("assets/minecraft/textures/block/stone.png", FileOptions::default()).unwrap();
            zw.write_all(b"png").unwrap();
            zw.finish().unwrap();
        }
        assert!(is_valid_vanilla_jar(&vanilla_jar));
        let _ = std::fs::remove_dir_all(&tmp);

        // uvlock propagation & fluid flow side textures
        let mut assets = AssetIndex::empty();
        assets.insert_entry(
            "assets/minecraft/blockstates/oak_stairs.json",
            br#"{"variants":{"facing=east":{"model":"minecraft:block/oak_stairs","y":90,"uvlock":true}}}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/minecraft/models/block/oak_stairs.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"#top"}}}]}"##.to_vec(),
        );
        let stairs = make_state("minecraft:oak_stairs", &[("facing", "east")]);
        match assets.resolve_block_model(&stairs) {
            ResolvedBlockModel::Elements(els) => {
                assert!(!els.is_empty());
                assert!(els[0].uvlock, "uvlock: true from blockstate variant must propagate to ModelElement");
            }
            other => panic!("Expected Elements, got {:?}", other),
        }

        let water = make_state("minecraft:water", &[("level", "0")]);
        match assets.resolve_block_model(&water) {
            ResolvedBlockModel::Elements(els) => {
                assert_eq!(els[0].faces.get(&FaceDir::North).unwrap().texture, "minecraft:block/water_flow");
                assert_eq!(els[0].faces.get(&FaceDir::Up).unwrap().texture, "minecraft:block/water_still");
            }
            other => panic!("Expected fluid Elements, got {:?}", other),
        }
    }

    #[test]
    fn test_atlas_tint_buckets_and_16_dye_signs() {
        use crate::atlas::{
            build_sign_atlas_with_assets, classify_texture_bucket, effective_biome_tint, RenderBucket,
        };

        // 1. Redstone dust & stems must NOT be tinted green; lily_pad gets fixed [72, 181, 24]
        assert_eq!(effective_biome_tint("minecraft:redstone_wire", "minecraft:block/redstone_dust_dot", Some(0), "minecraft:plains"), None);
        assert_eq!(effective_biome_tint("minecraft:melon_stem", "minecraft:block/melon_stem", Some(0), "minecraft:plains"), None);
        assert_eq!(effective_biome_tint("minecraft:lily_pad", "minecraft:block/lily_pad", Some(0), "minecraft:desert"), Some([72, 181, 24]));

        // 2. Coral blocks and bamboo blocks must NOT be misclassified into Translucent/Cutout
        let opaque_img = ImageBuffer::from_pixel(4, 4, Rgba([200, 120, 180, 255]));
        assert_eq!(classify_texture_bucket("minecraft:block/bubble_coral_block", &opaque_img), RenderBucket::TerrainOpaque);
        assert_eq!(classify_texture_bucket("minecraft:block/bamboo_block", &opaque_img), RenderBucket::TerrainOpaque);
        assert_eq!(classify_texture_bucket("minecraft:block/bamboo_planks", &opaque_img), RenderBucket::TerrainOpaque);
        assert_eq!(classify_texture_bucket("minecraft:block/bamboo_mosaic", &opaque_img), RenderBucket::TerrainOpaque);

        // 3. Sign atlas with all 16 dye colors + long line scaling
        let assets = AssetIndex::empty();
        let font = FontRenderer::from_assets(&assets);
        let mut signs = HashMap::new();
        signs.insert(
            (0, 64, 0),
            crate::anvil::SignData {
                x: 0,
                y: 64,
                z: 0,
                front_lines: [
                    "VeryLongSignTextLineExceeding14Chars".to_string(),
                    "Cyan Dye".to_string(),
                    String::new(),
                    String::new(),
                ],
                back_lines: Default::default(),
                front_color: "cyan".to_string(),
                back_color: "magenta".to_string(),
            },
        );
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 0, max_y: 64, max_z: 0 },
            palette: vec![make_state("minecraft:oak_sign", &[])],
            air_id: 99,
            blocks: vec![0],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0],
            signs,
            player_spawn_mc: None,
        };
        let sign_atlas = build_sign_atlas_with_assets(&grid, &assets, &font);
        assert!(sign_atlas.rects.contains_key("0,64,0,front"));
    }

    #[test]
    fn test_collision_passable_partial_boxes_snow_and_zones() {
        use crate::collision::build_world_collision_with_chunk_size;

        // 1. False-positive passables fixed
        assert!(!is_passable_block(&make_state("minecraft:bubble_coral_block", &[])));
        assert!(!is_passable_block(&make_state("minecraft:red_mushroom_block", &[])));
        assert!(!is_passable_block(&make_state("minecraft:mushroom_stem", &[])));
        assert!(!is_passable_block(&make_state("minecraft:campfire", &[("lit", "true")])));

        // 2. Scaffolding & new passables
        assert!(is_passable_block(&make_state("minecraft:scaffolding", &[])));
        assert!(is_passable_block(&make_state("minecraft:powder_snow", &[])));
        assert!(is_passable_block(&make_state("minecraft:lily_pad", &[])));

        // 3. PartialBox sub-voxel collisions
        let lantern_boxes = block_local_collision_boxes(&make_state("minecraft:lantern", &[]));
        assert!(lantern_boxes[0].0[0] > 0.0 && lantern_boxes[0].0[3] < 1.0, "Lantern must be sub-voxel");
        let soul_sand = block_local_collision_boxes(&make_state("minecraft:soul_sand", &[]));
        assert!((soul_sand[0].0[4] - 0.875).abs() < 1e-6);

        // 4. Snow layers == 8 must be 1.0
        assert_eq!(snow_collision_height(8), Some(1.0));

        // 5. Distinct packed_ice / blue_ice, waterlogged, and lit=false campfire filtering
        let air = make_state("minecraft:air", &[]);
        let packed = make_state("minecraft:packed_ice", &[]);
        let blue = make_state("minecraft:blue_ice", &[]);
        let unlit_camp = make_state("minecraft:campfire", &[("lit", "false")]);
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 2, max_y: 65, max_z: 0 },
            palette: vec![air, packed, blue, unlit_camp],
            air_id: 0,
            blocks: vec![1, 2, 3, 0, 0, 0],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 6],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision_with_chunk_size(&grid, [0.0, 64.0, 0.0], 39.37, 64);
        assert!(col.physics_zones.iter().any(|z| z.kind == "packed_ice"));
        assert!(col.physics_zones.iter().any(|z| z.kind == "blue_ice"));
        assert!(col.hazard_boxes.is_empty(), "Unlit campfire must not emit hazard box");
    }

    #[test]
    fn test_mesher_culling_fence_gates_buttons_and_open_doors() {
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        // Leaves-to-leaves & ice-to-ice culling
        let leaves = make_state("minecraft:oak_leaves", &[]);
        assert!(should_cull_face(&leaves, &leaves, FaceDir::North));
        let ice = make_state("minecraft:ice", &[]);
        assert!(should_cull_face(&ice, &ice, FaceDir::East));

        // Fence gate + proximity-linked button
        let air = make_state("minecraft:air", &[]);
        let gate = make_state("minecraft:oak_fence_gate", &[("facing", "north"), ("open", "false")]);
        let btn = make_state("minecraft:stone_button", &[("facing", "north"), ("face", "wall")]);
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 1, max_y: 64, max_z: 0 },
            palette: vec![air.clone(), gate, btn],
            air_id: 0,
            blocks: vec![1, 2],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 2],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty; grid.palette.len()];
        let (feat, _reqs) = extract_features_and_requests(&grid, &models, [0.0, 64.0, 0.0], 39.37);
        assert_eq!(feat.doors.len(), 1);
        assert_eq!(feat.doors[0].short_type, "oak_fence_gate");
        assert_ne!(feat.doors[0].mat_name, "atlas_brush");
        assert_eq!(feat.buttons.len(), 1);
        assert_eq!(feat.buttons[0].target_ids, vec!["mc_door_0_oak_fence_gate".to_string()]);

        // mesh_world_chunks_with_chunk_size accepts custom chunk size without panic
        let mut assets = AssetIndex::empty();
        assets.insert_entry(
            "assets/minecraft/blockstates/stone.json",
            br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/minecraft/models/block/stone.json",
            br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#.to_vec(),
        );
        let stone_img = ImageBuffer::from_pixel(16, 16, Rgba([128, 128, 128, 255]));
        assets.insert_entry("assets/minecraft/textures/block/stone.png", encode_png_bytes(&stone_img));
        let stone_bs = make_state("minecraft:stone", &[]);
        let stone_grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 0, max_y: 64, max_z: 0 },
            palette: vec![air.clone(), stone_bs.clone()],
            air_id: 0,
            blocks: vec![1],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let stone_models = vec![ResolvedBlockModel::Empty, assets.resolve_block_model(&stone_bs)];
        let (stone_feat, stone_reqs) = extract_features_and_requests(&stone_grid, &stone_models, [0.0, 64.0, 0.0], 39.37);
        let atlas = build_texture_atlas(&stone_reqs, &assets, 4);
        let font = FontRenderer::from_assets(&assets);
        let sign_atlas = crate::atlas::build_sign_atlas_with_assets(&stone_grid, &assets, &font);
        let chunks = mesh_world_chunks_with_chunk_size(&stone_grid, &stone_models, &atlas, &sign_atlas, &stone_feat, [0.0, 64.0, 0.0], 39.37, 16);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_smd_export_name_prefix_translucent_qc_and_walkable_priority() {
        use crate::mesher::ChunkMesh;
        use crate::smd_writer::emit_chunk_smds_and_qcs;

        let tmp = std::env::temp_dir().join(format!("mcgm_test_smd_fix_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let tri_trans = MeshTri {
            material: "terrain_translucent".into(),
            pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
            norm: [[0.0, 0.0, 1.0]; 3],
            uv: [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9]],
        };
        let cm = ChunkMesh {
            cx: 0,
            cz: 0,
            name: "chunk_0_0".to_string(),
            origin_source: [0.0, 0.0, 0.0],
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [39.37, 39.37, 39.37],
            solid_tris: vec![],
            translucent_tris: vec![tri_trans],
            has_cutout: false,
        };
        let col = CollisionBox {
            box_coords: [0.0, 0.0, 0.0, 39.37, 39.37, 39.37],
            block_class: "full".into(),
            is_walkable: true,
            is_opaque_roof: true,
            surface_prop: SurfaceProp::Rock,
        };
        let art = emit_chunk_smds_and_qcs(&cm, &[col], &tmp, "custom_mc", 2048, "castle_map").unwrap();
        let t_rel = art
            .translucent_model_rel_path
            .as_deref()
            .expect("translucent_model_rel_path must be set");
        assert!(t_rel.contains("castle_map_chunk_0_0_translucent"));
        assert!(!art.overflow_boxes.is_empty(), "Translucent-only chunk must preserve collision via overflow_boxes");
        let tqc_path = art.translucent_qc_path.expect("translucent QC must be emitted");
        let tqc = std::fs::read_to_string(&tqc_path).unwrap();
        assert!(!tqc.contains("$mostlyopaque"), "Translucent-only QC must omit $mostlyopaque");

        // Walkable full cube must outrank non-walkable full cube
        let phys_path = tmp.join("prio_walkable.smd");
        let boxes = vec![
            CollisionBox {
                box_coords: [0.0, 0.0, 0.0, 39.37, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: false,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            },
            CollisionBox {
                box_coords: [39.37, 0.0, 0.0, 78.74, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            },
        ];
        let overflow = write_physics_smd(&boxes, &phys_path, [0.0, 0.0, 0.0], 1, 0.015).unwrap();
        assert_eq!(overflow.len(), 1);
        assert!(!overflow[0].is_walkable, "Non-walkable cube must overflow before walkable cube");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_glua_convars_block_physics_smooth_fall_and_no_truncation() {
        use crate::collision::PhysicsZone;
        use crate::glua::generate_mcgm_smooth_fall_autorun;

        let lua = generate_mcgm_physics_autorun();
        for cvar in [
            "mcgm_view_bobbing",
            "mcgm_sprint_mode",
            "mcgm_hold_jump_bhop",
            "mcgm_smooth_step_cam",
            "mcgm_swap_sneak_sprint",
            "mcgm_climb_parity",
        ] {
            assert!(lua.contains(cvar), "Missing ConVar {}", cvar);
        }
        assert!(lua.contains("packed_ice"));
        assert!(lua.contains("blue_ice"));
        assert!(lua.contains("soul_sand"));
        assert!(lua.contains("honey"));
        assert!(lua.contains("magma"));
        assert!(lua.contains("CalcView"));
        assert!(lua.contains("PopulateToolMenu"));
        assert!(lua.contains("RevertPlayerDefaults"));
        assert!(lua.contains("ply:GetMoveType() == MOVETYPE_WALK"));
        assert!(!lua.contains("if cv_smooth_step_cam and game.SinglePlayer() then"));
        assert!(lua.contains("elseif inClimb and not onGround then"));
        assert!(lua.contains("elseif (mv:KeyDown(IN_JUMP) or fSpd > 10) and not onGround then"));

        let sf = generate_mcgm_smooth_fall_autorun();
        assert!(sf.contains("IsMinecraftMap"), "mcgm_smooth_fall.lua must be gated on IsMinecraftMap");

        // No .take(400) truncation on PhysicsZone emission
        let zones: Vec<PhysicsZone> = (0..450)
            .map(|i| PhysicsZone {
                kind: "water".to_string(),
                min: [i as f64, 0.0, 0.0],
                max: [(i + 1) as f64, 39.37, 39.37],
            })
            .collect();
        let map_lua = generate_map_physics_lua("gm_mc_many_zones", 39.37, true, &zones);
        assert!(map_lua.contains("449"), "All 450 zones must be emitted without 400-zone truncation");
    }

    #[test]
    fn test_vmf_iron_door_button_io_and_vmt_model_flag() {
        let tmp = std::env::temp_dir().join(format!("mcgm_test_vmf_io_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let vmt_path = tmp.join("model.vmt");
        generate_vmt(&vmt_path, "VertexLitGeneric", "custom_mc/atlas", "dirt", false, false, false, false, false).unwrap();
        let vmt = std::fs::read_to_string(&vmt_path).unwrap();
        assert!(vmt.contains("\"$model\" \"1\""), "VertexLitGeneric VMT must include $model 1");

        let vmf_path = tmp.join("io.vmf");
        let iron_door = InteractableDoor {
            mc_pos: [0, 64, 0],
            box_coords: [0.0, 0.0, 0.0, 39.37, 7.38, 78.74],
            origin: [3.69, 3.69, 39.37],
            facing: "north".to_string(),
            hinge: "left".to_string(),
            open: false,
            spawnflags: 306, // includes 256 (Use Opens), which VMF builder must strip for iron doors
            mat_name: "door_iron_door".to_string(),
            short_type: "iron_door".to_string(),
        };
        let btn = crate::mesher::InteractableButton {
            kind: "button".to_string(),
            box_coords: [0.0, 40.0, 20.0, 10.0, 50.0, 30.0],
            origin: [5.0, 45.0, 25.0],
            facing: "north".to_string(),
            face: "wall".to_string(),
            move_dir: [0.0, 0.0, 1.0],
            target_ids: vec!["mc_door_0_iron_door".to_string()],
            powered: false,
            material: "atlas_brush".to_string(),
        };
        build_vmf(&VmfBuildParams {
            output_vmf: &vmf_path,
            category: "custom_mc",
            bounds_min: [-200.0, -200.0, 0.0],
            bounds_max: [200.0, 200.0, 200.0],
            chunks: &[],
            all_solid_boxes: &[],
            ladder_boxes: &[],
            water_pools: &[],
            hazard_boxes: &[],
            core_occluders: &[],
            doors: &[iron_door],
            buttons: &[btn],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 48.0]],
            lighting_style: "minecraft",
            hq_light: false,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();
        let vmf_text = std::fs::read_to_string(&vmf_path).unwrap();
        assert!(vmf_contains_kv(&vmf_text, "spawnflags", "50"), "Iron door must strip bit 256 (306 -> 50)");
        assert!(vmf_text.contains("door_metal_medium_open1.wav"));
        assert!(vmf_text.contains("connections"));
        assert!(vmf_text.contains("mc_door_0_iron_door,Toggle"));
        assert!(vmf_contains_kv(&vmf_text, "_minlight", "0.30"), "Door/button must emit _minlight so interior faces are never pitch black");
        assert!(vmf_contains_kv(&vmf_text, "vrad_brush_cast_shadows", "0"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_bell_body_water_farmland_culling_clean_thumbnail_and_live_physics_toggle() {
        use crate::assets::{AssetIndex, ResolvedBlockModel};
        use crate::compiler::{generate_map_thumbnail, render_orthographic_map_preview};
        use crate::glua::{generate_mcgm_physics_autorun, generate_mcgm_smooth_fall_autorun};

        // 1. Bell must include hanging golden bell body + flared rim elements
        let assets = AssetIndex::empty();
        let bell_bs = BlockState::new("minecraft:bell".to_string(), BTreeMap::new());
        match assets.resolve_block_model(&bell_bs) {
            ResolvedBlockModel::Elements(els) => {
                assert!(
                    els.iter().any(|e| e.from == [5.0, 6.0, 5.0] && e.to == [11.0, 13.0, 11.0]),
                    "Bell must emit upper hanging body element"
                );
                assert!(
                    els.iter().any(|e| e.from == [4.0, 4.0, 4.0] && e.to == [12.0, 6.0, 12.0]),
                    "Bell must emit lower flared rim element"
                );
            }
            _ => panic!("Bell must resolve to Elements including bell body"),
        }

        // 2. Water side face adjacent to farmland / dirt_path must be culled (prevents Z-fighting)
        let water = BlockState::new("minecraft:water".to_string(), BTreeMap::new());
        let farmland = BlockState::new("minecraft:farmland".to_string(), BTreeMap::new());
        let dirt_path = BlockState::new("minecraft:dirt_path".to_string(), BTreeMap::new());
        assert!(should_cull_face(&water, &farmland, FaceDir::East));
        assert!(should_cull_face(&water, &dirt_path, FaceDir::North));
        assert!(!should_cull_face(&water, &farmland, FaceDir::Up));

        // 3. Thumbnails must not draw a dark bottom title bar / map name overlay
        let tmp = std::env::temp_dir().join(format!("mcgm_test_clean_thumb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        generate_map_thumbnail("gm_clean_view", &tmp, None, "day").unwrap();
        let thumb_img = image::open(tmp.join("maps/thumb/gm_clean_view.png")).unwrap().to_rgba8();
        // Bottom region (y=470) should be dirt color [134, 96, 67], not dark title bar [16, 18, 24]
        let p = thumb_img.get_pixel(256, 470).0;
        assert!(p[0] > 100 && p[1] > 70, "Thumbnail bottom must be terrain view, not dark title bar: {:?}", p);

        let coords = WorldCoords { min_x: 0, min_y: 0, min_z: 0, max_x: 2, max_y: 2, max_z: 2 };
        let air = BlockState::new("minecraft:air".to_string(), BTreeMap::new());
        let grass = BlockState::new("minecraft:grass_block".to_string(), BTreeMap::new());
        let mut blocks = vec![0u16; 27];
        blocks[13] = 1;
        let grid = VoxelGrid {
            coords,
            palette: vec![air, grass],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0u16; 27],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let ortho = render_orthographic_map_preview(&grid, &assets, "gm_clean_view", "day");
        // Bottom row inside border (y=480) should be bright day sky, not dark title bar
        let op = ortho.get_pixel(40, 480).0;
        assert!(op[0] > 100 && op[2] > 180, "Orthographic preview must have no bottom title bar: {:?}", op);
        let _ = std::fs::remove_dir_all(&tmp);

        // 4. GLua: gentle 1-block walk-off fall, smooth sprint bobbing, and instant live physics toggle
        let lua = generate_mcgm_physics_autorun();
        assert!(lua.contains("from_jump"));
        assert!(lua.contains("0.50 * mcGrav"));
        assert!(lua.contains("OnPlayerHitGround"));
        assert!(lua.contains("_mcgmBobBlend"));
        assert!(lua.contains("mcgm_toggle_physics"));
        assert!(lua.contains("!mcphysics"));
        assert!(lua.contains("SnapshotPlayerDefaults"));
        assert!(lua.contains("ShouldSuppressSmoothFall"));

        let sf = generate_mcgm_smooth_fall_autorun();
        assert!(sf.contains("IsSmoothFallAllowed"));

        // 5. Overflow collision boxes (> max_convex_pieces) must split into extra_collision_models with 0 dropped boxes
        let tmp_col = std::env::temp_dir().join(format!("mcgm_test_extra_col_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_col);
        std::fs::create_dir_all(&tmp_col).unwrap();
        let cm = crate::mesher::ChunkMesh {
            name: "chunk_0_0".to_string(),
            cx: 0,
            cz: 0,
            origin_source: [0.0, 0.0, 0.0],
            solid_tris: vec![MeshTri {
                material: "atlas_opaque".into(),
                pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                norm: [[0.0, 0.0, 1.0]; 3],
                uv: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            }],
            translucent_tris: Vec::new(),
            has_cutout: false,
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [100.0, 100.0, 100.0],
        };
        let boxes: Vec<CollisionBox> = (0..10)
            .map(|i| CollisionBox {
                box_coords: [i as f64 * 10.0, 0.0, 0.0, (i + 1) as f64 * 10.0, 10.0, 10.0],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            })
            .collect();
        let art = crate::smd_writer::emit_chunk_smds_and_qcs(&cm, &boxes, &tmp_col, "custom_mc", 4, "t").unwrap();
        assert!(art.overflow_boxes.is_empty(), "Overflow boxes should be split into extra collision models");
        assert_eq!(art.extra_collision_models.len(), 2, "10 boxes with max 4 -> main(4) + col1(4) + col2(2)");
        assert!(art.extra_collision_models[0].0.is_file());
        assert!(art.extra_collision_models[1].0.is_file());
        let _ = std::fs::remove_dir_all(&tmp_col);
    }

    #[test]
    fn test_small_rock_and_model_aabb_collision() {
        use crate::assets::{FaceDir, ModelElement, ModelFace, ResolvedBlockModel};
        use crate::collision::{build_world_collision_with_models, is_passable_block};

        let rock = make_state("bigglobe:rock", &[("rocks", "1")]);
        assert!(is_passable_block(&rock));

        // Unknown modded pebble-sized block with small Elements AABB (3x2x3 MC pixels)
        let custom_pebble = make_state("somemod:tiny_artifact", &[]);
        let mut faces = HashMap::new();
        faces.insert(
            FaceDir::Up,
            ModelFace {
                uv: None,
                texture: "minecraft:block/stone".to_string(),
                rotation: 0,
                tint_index: None,
                cullface: None,
            },
        );
        let pebble_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [6.0, 0.0, 6.0],
            to: [9.0, 2.0, 9.0],
            rotation: None,
            faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);

        let air = make_state("minecraft:air", &[]);
        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 1,
            max_y: 65,
            max_z: 1,
        };
        let grid = VoxelGrid {
            coords,
            palette: vec![air, custom_pebble],
            air_id: 0,
            blocks: vec![1; coords.size_x() * coords.size_y() * coords.size_z()],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; coords.size_x() * coords.size_y() * coords.size_z()],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty, pebble_model];
        let col = build_world_collision_with_models(&grid, Some(&models), [0.0, 64.0, 0.0], 39.37, 16);
        assert!(
            col.all_solid_boxes.is_empty(),
            "Small floor-clutter model AABB should be passable, got {:?}",
            col.all_solid_boxes
        );
    }

    #[test]
    fn test_multiframe_vtf_header_and_payload() {
        use crate::compiler::write_multiframe_vtf;
        let tmp = std::env::temp_dir().join(format!("mcgm_test_vtf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let vtf_path = tmp.join("anim_fire.vtf");

        let f0 = ImageBuffer::from_pixel(64, 64, Rgba([255, 120, 20, 255]));
        let f1 = ImageBuffer::from_pixel(64, 64, Rgba([255, 200, 40, 200]));
        let f2 = ImageBuffer::from_pixel(64, 64, Rgba([220, 80, 10, 255]));
        write_multiframe_vtf(&vtf_path, &[f0, f1, f2]).unwrap();

        let data = std::fs::read(&vtf_path).unwrap();
        assert_eq!(&data[0..4], b"VTF\0");
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(data[8..12].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(data[12..16].try_into().unwrap()), 80);
        assert_eq!(u16::from_le_bytes(data[16..18].try_into().unwrap()), 64);
        assert_eq!(u16::from_le_bytes(data[18..20].try_into().unwrap()), 64);
        assert_eq!(u32::from_le_bytes(data[20..24].try_into().unwrap()), 0x2001); // POINTSAMPLE | EIGHTBITALPHA
        assert_eq!(u16::from_le_bytes(data[24..26].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(data[52..56].try_into().unwrap()), 12); // BGRA8888
        assert_eq!(data[56], 7); // 64,32,16,8,4,2,1
        let mip_pixels = 64 * 64 + 32 * 32 + 16 * 16 + 8 * 8 + 4 * 4 + 2 * 2 + 1;
        let expected_len = 80 + 3 * mip_pixels * 4;
        assert_eq!(data.len(), expected_len);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_large_map_fog_farz_and_minecraft_self_shadow() {
        use crate::vmf_builder::{build_vmf, get_lighting_preset, VmfBuildParams};

        let preset = get_lighting_preset("minecraft");
        assert_eq!(preset.prop_disableselfshadowing, "0");
        assert_eq!(preset.fog_end, "8500");

        let tmp = std::env::temp_dir().join(format!("mcgm_test_farz_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let vmf_path = tmp.join("large.vmf");

        build_vmf(&VmfBuildParams {
            output_vmf: &vmf_path,
            category: "custom_mc",
            bounds_min: [-15000.0, -15000.0, 0.0],
            bounds_max: [15000.0, 15000.0, 2000.0],
            chunks: &[],
            all_solid_boxes: &[],
            ladder_boxes: &[],
            water_pools: &[],
            hazard_boxes: &[],
            core_occluders: &[],
            doors: &[],
            buttons: &[],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 64.0]],
            lighting_style: "minecraft",
            hq_light: true,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();

        let vmf_text = std::fs::read_to_string(&vmf_path).unwrap();
        assert!(vmf_contains_kv(&vmf_text, "farz", "9012"));
        assert!(!vmf_text.contains("func_viscluster"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_torch_light_priority_over_lava_and_clear_offset() {
        use crate::assets::ResolvedBlockModel;
        use crate::mesher::extract_features_and_requests;

        let air = make_state("minecraft:air", &[]);
        let lava = make_state("minecraft:lava", &[("level", "0")]);
        let torch = make_state("minecraft:torch", &[]);
        let wall_torch = make_state("minecraft:wall_torch", &[("facing", "east")]);

        // 40x66x40 grid with abundant deep lava at y=0 and torches at y=64
        let coords = WorldCoords {
            min_x: 0,
            min_y: 0,
            min_z: 0,
            max_x: 39,
            max_y: 65,
            max_z: 39,
        };
        let mut grid = VoxelGrid {
            coords,
            palette: vec![air, lava, torch, wall_torch],
            air_id: 0,
            blocks: vec![0; coords.size_x() * coords.size_y() * coords.size_z()],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; coords.size_x() * coords.size_y() * coords.size_z()],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        for x in 0..40 {
            for z in 0..40 {
                let idx = grid.index(x, 0, z).unwrap();
                grid.blocks[idx] = 1; // deep lava
            }
        }
        let t_idx = grid.index(5, 64, 5).unwrap();
        grid.blocks[t_idx] = 2; // standing torch
        let wt_idx = grid.index(20, 64, 20).unwrap();
        grid.blocks[wt_idx] = 3; // wall torch

        let models = vec![
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
        ];
        let scale = 39.37;
        let origin_mc = [20.0, 0.0, 20.0];
        let (feat, _) = extract_features_and_requests(&grid, &models, origin_mc, scale);

        let torch_lights: Vec<_> = feat.lights.iter().filter(|l| l.brightness == 380).collect();
        assert_eq!(torch_lights.len(), 2, "Both standing and wall torches must survive lava flooding");
        assert!(
            torch_lights.iter().all(|l| l.pos[2] > (64.70 * scale)),
            "Torch lights must sit cleanly above 10/16 mesh tip: {:?}",
            torch_lights.iter().map(|l| l.pos[2]).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_2d_cross_plant_dedup_and_upward_normal() {
        use crate::assets::{FaceDir, ModelElement, ModelFace, ModelRotation, ResolvedBlockModel};
        use crate::atlas::{build_texture_atlas, FontRenderer, SignAtlas};
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        let air = make_state("minecraft:air", &[]);
        let mushroom = make_state("minecraft:brown_mushroom", &[]);

        let make_cross_el = |angle: f32| {
            let mut faces = HashMap::new();
            let mf = |tex: &str| ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: tex.to_string(),
                rotation: 0,
                tint_index: None,
                cullface: None,
            };
            faces.insert(FaceDir::North, mf("minecraft:block/brown_mushroom"));
            faces.insert(FaceDir::South, mf("minecraft:block/brown_mushroom"));
            ModelElement {
                from: [0.8, 0.0, 8.0],
                to: [15.2, 16.0, 8.0],
                rotation: Some(ModelRotation {
                    origin: [8.0, 8.0, 8.0],
                    axis: 'y',
                    angle,
                    rescale: true,
                    euler_xyz: None,
                }),
                faces,
                rot_x: 0,
                rot_y: 0,
                rot_z: 0,
                uvlock: false,
            }
        };
        let mushroom_model = ResolvedBlockModel::Elements(vec![make_cross_el(45.0), make_cross_el(-45.0)]);

        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 0,
            max_y: 64,
            max_z: 0,
        };
        let grid = VoxelGrid {
            coords,
            palette: vec![air, mushroom],
            air_id: 0,
            blocks: vec![1],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty, mushroom_model];
        let (feat, reqs) = extract_features_and_requests(&grid, &models, [0.5, 64.0, 0.5], 39.37);
        let assets = AssetIndex::empty();
        let atlas = build_texture_atlas(&reqs, &assets, 4);
        let sign_atlas = SignAtlas {
            image: ImageBuffer::new(16, 16),
            rects: HashMap::new(),
        };
        let _ = FontRenderer::from_assets(&assets);
        let chunks = mesh_world_chunks_with_chunk_size(&grid, &models, &atlas, &sign_atlas, &feat, [0.5, 64.0, 0.5], 39.37, 16);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].solid_tris.len(), 4, "2 $nocull planes = 4 tris (no duplicate back-to-back Z-fighting tris)");
        for tri in &chunks[0].solid_tris {
            for n in &tri.norm {
                assert_eq!(*n, [0.0, 0.0, 1.0], "2D cutout plant normal must point straight up");
            }
        }
    }

    #[test]
    fn test_lz4_java_block_decompression() {
        use crate::anvil::decompress_lz4_java_blocks;

        let mut stream = Vec::new();
        // Block 1: RAW (0x10), 3 bytes "ABC"
        stream.extend_from_slice(b"LZ4Block");
        stream.push(0x10);
        stream.extend_from_slice(&3u32.to_le_bytes());
        stream.extend_from_slice(&3u32.to_le_bytes());
        stream.extend_from_slice(&0u32.to_le_bytes()); // checksum placeholder
        stream.extend_from_slice(b"ABC");

        // Block 2: LZ4 (0x20), literal "X" (len 1) + match offset=1, match_len=5 (1 + 4) -> "XXXXXX" (6 bytes)
        let lz4_block = [0x11u8, b'X', 0x01, 0x00];
        stream.extend_from_slice(b"LZ4Block");
        stream.push(0x20);
        stream.extend_from_slice(&(lz4_block.len() as u32).to_le_bytes());
        stream.extend_from_slice(&6u32.to_le_bytes());
        stream.extend_from_slice(&0u32.to_le_bytes());
        stream.extend_from_slice(&lz4_block);

        // End-of-stream block
        stream.extend_from_slice(b"LZ4Block");
        stream.push(0x10);
        stream.extend_from_slice(&0u32.to_le_bytes());
        stream.extend_from_slice(&0u32.to_le_bytes());
        stream.extend_from_slice(&0u32.to_le_bytes());

        let out = decompress_lz4_java_blocks(&stream).expect("LZ4Block stream should decompress");
        assert_eq!(out, b"ABCXXXXXX");
    }

    #[test]
    fn test_level_dat_1_21_9_spawn_and_version() {
        use crate::anvil::{read_level_spawn, read_level_version};
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;

        let tmp = std::env::temp_dir().join("mcgm_test_1_21_9_level");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut ver_map = HashMap::new();
        ver_map.insert("Name".to_string(), fastnbt::Value::String("1.21.9".to_string()));
        ver_map.insert("Id".to_string(), fastnbt::Value::Int(4556));

        let mut spawn_map = HashMap::new();
        spawn_map.insert(
            "pos".to_string(),
            fastnbt::Value::IntArray(fastnbt::IntArray::new(vec![120, 72, -45])),
        );

        let mut data_map = HashMap::new();
        data_map.insert("DataVersion".to_string(), fastnbt::Value::Int(4556));
        data_map.insert("Version".to_string(), fastnbt::Value::Compound(ver_map));
        data_map.insert("spawn".to_string(), fastnbt::Value::Compound(spawn_map));

        let mut root_map = HashMap::new();
        root_map.insert("Data".to_string(), fastnbt::Value::Compound(data_map));
        let nbt_bytes = fastnbt::to_bytes(&fastnbt::Value::Compound(root_map)).unwrap();

        let level_dat = tmp.join("level.dat");
        let f = std::fs::File::create(&level_dat).unwrap();
        let mut gz = GzEncoder::new(f, Compression::default());
        gz.write_all(&nbt_bytes).unwrap();
        gz.finish().unwrap();

        let ver = read_level_version(&tmp).expect("version should be read");
        assert_eq!(ver.name.as_deref(), Some("1.21.9"));
        assert_eq!(ver.data_version, Some(4556));

        let spawn = read_level_spawn(&tmp).expect("1.21.9 Data.spawn.pos should be read");
        assert_eq!(spawn, [120.0, 72.0, -45.0]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_mc_version_tuple() {
        use crate::assets::parse_mc_version_tuple;

        assert_eq!(parse_mc_version_tuple("1.20.1"), Some((1, 20, 1)));
        assert_eq!(parse_mc_version_tuple("1.21"), Some((1, 21, 0)));
        assert_eq!(parse_mc_version_tuple("1.21.5.jar"), Some((1, 21, 5)));
        assert_eq!(parse_mc_version_tuple("1.21.11"), Some((1, 21, 11)));
        assert_eq!(parse_mc_version_tuple("26.1"), Some((26, 1, 0)));
        assert_eq!(
            parse_mc_version_tuple("fabric-loader-0.16.10-1.21.4.jar"),
            Some((1, 21, 4))
        );
        assert_eq!(parse_mc_version_tuple("fabric-loader-0.16.10"), None);
        assert_eq!(parse_mc_version_tuple("client"), None);
    }

    #[test]
    fn test_1_21_plus_blocks_and_euler_model_json() {
        use crate::assets::{AssetIndex, ResolvedBlockModel};
        use crate::collision::is_passable_block;
        use crate::mesher::{extract_features_and_requests, is_full_opaque_occluder};

        for name in [
            "minecraft:copper_grate",
            "minecraft:exposed_copper_grate",
            "minecraft:copper_bars",
            "minecraft:copper_chain",
            "minecraft:oak_shelf",
            "minecraft:copper_golem_statue",
        ] {
            let st = make_state(name, &[]);
            assert!(!is_full_opaque_occluder(&st), "{} must not be full opaque occluder", name);
        }

        for name in [
            "minecraft:bush",
            "minecraft:firefly_bush",
            "minecraft:pale_hanging_moss",
            "minecraft:open_eyeblossom",
            "minecraft:resin_clump",
            "minecraft:wildflowers",
            "minecraft:cactus_flower",
            "minecraft:pale_moss_carpet",
        ] {
            let st = make_state(name, &[]);
            assert!(is_passable_block(&st), "{} should be passable", name);
        }

        // Lit copper bulb and copper torch should emit block lights
        let air = make_state("minecraft:air", &[]);
        let bulb = make_state("minecraft:copper_bulb", &[("lit", "true")]);
        let ox_bulb = make_state("minecraft:oxidized_copper_bulb", &[("lit", "true")]);
        let ctorch = make_state("minecraft:copper_torch", &[]);
        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 2,
            max_y: 64,
            max_z: 0,
        };
        let grid = VoxelGrid {
            coords,
            palette: vec![air, bulb, ox_bulb, ctorch],
            air_id: 0,
            blocks: vec![1, 2, 3],
            biome_palette: vec!["minecraft:pale_garden".to_string()],
            biomes: vec![0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
            ResolvedBlockModel::Empty,
        ];
        let (feat, _) = extract_features_and_requests(&grid, &models, [1.5, 64.0, 0.5], 39.37);
        assert!(!feat.lights.is_empty(), "1.21+ copper lights should be emitted");

        // 1.21.11+ multi-axis model rotation { origin, x, y, z }
        let mut assets = AssetIndex::empty();
        assets.insert_entry(
            "assets/minecraft/blockstates/test_euler.json",
            br#"{"variants":{"":{"model":"minecraft:block/test_euler"}}}"#.to_vec(),
        );
        assets.insert_entry(
            "assets/minecraft/models/block/test_euler.json",
            br##"{
                "textures": {"all": "minecraft:block/stone"},
                "elements": [{
                    "from": [4, 4, 4],
                    "to": [12, 12, 12],
                    "rotation": {"origin": [8, 8, 8], "x": 22.5, "y": 45.0, "z": 0.0},
                    "faces": {"up": {"texture": "#all"}}
                }]
            }"##
            .to_vec(),
        );
        let resolved = assets.resolve_block_model(&make_state("minecraft:test_euler", &[]));
        match resolved {
            ResolvedBlockModel::Elements(els) => {
                assert_eq!(els.len(), 1);
                let rot = els[0].rotation.as_ref().expect("rotation should deserialize");
                assert_eq!(rot.euler_xyz, Some([22.5, 45.0, 0.0]));
            }
            _ => panic!("Expected Elements for 1.21.11 euler rotation model"),
        }
    }

    #[test]
    fn test_detect_tools_strips_legacy_dead_keys() {
        use crate::compiler::detect_tools_and_save;
        let tmp_dir = std::env::temp_dir().join(format!("mcgm_cfg_strip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let cfg_path = tmp_dir.join("config.json");
        std::fs::write(
            &cfg_path,
            r#"{
  "blender_path": "C:\\Blender\\blender.exe",
  "mineways_path": "C:\\Mineways\\Mineways.exe",
  "jdk_path": "C:\\Java\\javac.exe",
  "scale_factor": 42.0,
  "max_convex_pieces": 128
}"#,
        )
        .unwrap();

        let _ = detect_tools_and_save(&cfg_path).unwrap();
        let saved = std::fs::read_to_string(&cfg_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert!(val.get("blender_path").is_none());
        assert!(val.get("mineways_path").is_none());
        assert!(val.get("jdk_path").is_none());
        assert_eq!(val.get("scale_factor").and_then(|v| v.as_f64()), Some(42.0));
        assert_eq!(val.get("max_convex_pieces").and_then(|v| v.as_u64()), Some(128));
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_features_parallel_multi_y_deterministic() {
        use crate::assets::{FaceDir, ModelElement, ModelFace};
        use crate::mesher::extract_features_and_requests;
        use std::collections::HashMap;

        let air = make_state("minecraft:air", &[]);
        let stone = make_state("minecraft:stone", &[]);
        let grass = make_state("minecraft:grass_block", &[]);
        let door_low = make_state(
            "minecraft:oak_door",
            &[("half", "lower"), ("facing", "north"), ("hinge", "left"), ("open", "false")],
        );
        let door_up = make_state(
            "minecraft:oak_door",
            &[("half", "upper"), ("facing", "north"), ("hinge", "left"), ("open", "false")],
        );
        let btn = make_state(
            "minecraft:stone_button",
            &[("facing", "south"), ("face", "wall"), ("powered", "false")],
        );
        let torch = make_state("minecraft:torch", &[]);
        let water = make_state("minecraft:water", &[("level", "0")]);
        let gate = make_state(
            "minecraft:oak_fence_gate",
            &[("facing", "east"), ("open", "false")],
        );

        let palette = vec![air, stone, grass, door_low, door_up, btn, torch, water, gate];
        let coords = WorldCoords {
            min_x: 0,
            min_y: 60,
            min_z: 0,
            max_x: 4,
            max_y: 64,
            max_z: 4,
        };
        let sx = coords.size_x();
        let sy = coords.size_y();
        let sz = coords.size_z();
        let mut blocks = vec![0u16; sx * sy * sz];
        let mut set_blk = |x: i32, y: i32, z: i32, pid: u16| {
            let idx = ((y - coords.min_y) as usize * sz + (z - coords.min_z) as usize) * sx
                + (x - coords.min_x) as usize;
            blocks[idx] = pid;
        };
        for x in 0..=4 {
            for z in 0..=4 {
                set_blk(x, 60, z, 1); // stone floor
            }
        }
        set_blk(0, 61, 0, 2); // grass
        set_blk(1, 61, 1, 3); // door lower
        set_blk(1, 62, 1, 4); // door upper
        set_blk(2, 61, 1, 5); // button next to door
        set_blk(4, 63, 4, 6); // torch on high Y
        set_blk(3, 61, 3, 7); // flat water enclosed on all 4 sides? wait: surround with stone
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            set_blk(3 + dx, 61, 3 + dz, 1);
        }
        set_blk(0, 64, 4, 8); // fence gate

        let grid = VoxelGrid {
            coords,
            palette,
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string(), "minecraft:desert".to_string()],
            biomes: vec![0; (sx / 4 + 1) * (sy / 4 + 1) * (sz / 4 + 1)],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };

        let mut grass_faces = HashMap::new();
        grass_faces.insert(
            FaceDir::Up,
            ModelFace {
                uv: None,
                texture: "minecraft:block/grass_block_top".to_string(),
                rotation: 0,
                tint_index: Some(0),
                cullface: Some(FaceDir::Up),
            },
        );
        let grass_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.0, 0.0, 0.0],
            to: [16.0, 16.0, 16.0],
            rotation: None,
            faces: grass_faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);
        let mut models = vec![ResolvedBlockModel::Empty; grid.palette.len()];
        models[2] = grass_model;

        let (f1, r1) = extract_features_and_requests(&grid, &models, [2.5, 60.0, 2.5], 39.37);
        let (f2, r2) = extract_features_and_requests(&grid, &models, [2.5, 60.0, 2.5], 39.37);

        assert_eq!(f1.doors.len(), 2);
        assert_eq!(f1.buttons.len(), 1);
        assert!(f1.flat_water_voxels.contains(&(3, 61, 3)));
        assert_eq!(f1.lights.len(), 1);
        assert_eq!(f1.doors[0].mc_pos, f2.doors[0].mc_pos);
        assert_eq!(f1.doors[1].mc_pos, f2.doors[1].mc_pos);
        assert_eq!(r1, r2);
        assert!(r1.iter().any(|k| k.tex_ref == "minecraft:block/grass_block_top" && k.tint_rgb.is_some()));
    }

    #[test]
    fn test_atlas_cache_hash_meta_and_vtfcmd_skip() {
        use crate::atlas::{
            build_texture_atlas, compute_atlas_cache_hash, load_atlas_cache_meta,
            save_atlas_cache_meta, TexRequestKey,
        };
        use crate::compiler::{compile_materials, ToolPaths};
        use std::collections::{HashMap, HashSet};

        let mut reqs = HashSet::new();
        reqs.insert(TexRequestKey {
            tex_ref: "minecraft:block/stone".to_string(),
            tint_rgb: None,
        });
        reqs.insert(TexRequestKey {
            tex_ref: "minecraft:block/grass_block_top".to_string(),
            tint_rgb: Some([120, 192, 88]),
        });

        let h1 = compute_atlas_cache_hash(&reqs, 39.37, "minecraft", 8);
        let h2 = compute_atlas_cache_hash(&reqs, 39.37, "minecraft", 8);
        let h_scale = compute_atlas_cache_hash(&reqs, 60.0, "minecraft", 8);
        let h_style = compute_atlas_cache_hash(&reqs, 39.37, "night", 8);
        assert_eq!(h1.len(), 8);
        assert_eq!(h1, h2);
        assert_ne!(h1, h_scale);
        assert_ne!(h1, h_style);

        let assets = AssetIndex::empty();
        let atlas = build_texture_atlas(&reqs, &assets, 8);

        let tmp = std::env::temp_dir().join(format!("mcgm_atlas_cache_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        let vtf_stage = work_dir.join("vtf_stage");
        std::fs::create_dir_all(&vtf_stage).unwrap();
        std::fs::create_dir_all(&addon_dir).unwrap();

        let atlas_png = work_dir.join(format!("atlas_{}.png", h1));
        let atlas_meta = work_dir.join(format!("atlas_{}.json", h1));
        let sign_png = work_dir.join("sign_atlas.png");
        crate::atlas::save_png_fast(&atlas.image, &atlas_png).unwrap();
        crate::atlas::save_png_fast(&image::RgbaImage::new(16, 16), &sign_png).unwrap();
        save_atlas_cache_meta(&atlas, &atlas_meta, 8).unwrap();

        let loaded = load_atlas_cache_meta(&atlas_meta).unwrap();
        assert_eq!(loaded.width, atlas.width);
        assert_eq!(loaded.height, atlas.height);
        assert_eq!(loaded.rects.len(), atlas.rects.len());

        // Pre-populate cached atlas_<hash8>.vtf
        let cached_atlas_vtf = vtf_stage.join(format!("atlas_{}.vtf", h1));
        std::fs::write(&cached_atlas_vtf, b"CACHED_VTF").unwrap();

        // Mock VTFCmd that logs every invocation and writes a dummy .vtf
        let calls_log = tmp.join("vtfcmd_calls.txt");
        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            format!(
                "@echo off\r\necho %* >> \"{}\"\r\nfor %%F in (%2) do echo DUMMY > \"%~4\\%%~nF.vtf\"\r\n",
                calls_log.display()
            ),
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 64,
        };
        compile_materials(
            &tools,
            "mcgm",
            &atlas_png,
            &sign_png,
            &HashMap::new(),
            &[],
            &addon_dir,
            false,
            true,
        )
        .unwrap();

        let log_txt = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert!(
            !log_txt.contains(&format!("atlas_{}.png", h1)),
            "VTFCmd must be skipped for cached atlas, got log: {}",
            log_txt
        );
        assert!(log_txt.contains("sign_atlas.png"));
        assert!(addon_dir
            .join("materials")
            .join("mcgm")
            .join("atlas_brush.vmt")
            .is_file());
        assert_eq!(
            std::fs::read(
                addon_dir
                    .join("materials")
                    .join("models")
                    .join("mcgm")
                    .join(format!("atlas_{}.vtf", h1))
            )
            .unwrap(),
            b"CACHED_VTF"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_bamboo_thin_collision_vs_bamboo_block_and_sapling() {
        use crate::collision::{block_local_collision_boxes, classify_solid_block, SolidClass};

        let bamboo = make_state("minecraft:bamboo", &[]);
        assert_eq!(classify_solid_block(&bamboo), SolidClass::PartialBox);
        let boxes = block_local_collision_boxes(&bamboo);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].0, [0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625]);
        assert_eq!(boxes[0].1, "partial");
        assert!(!boxes[0].2);

        let bamboo_block = make_state("minecraft:bamboo_block", &[]);
        assert_eq!(classify_solid_block(&bamboo_block), SolidClass::FullOpaque);
        assert_eq!(
            block_local_collision_boxes(&bamboo_block)[0].0,
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
        );

        let bamboo_sapling = make_state("minecraft:bamboo_sapling", &[]);
        assert_eq!(classify_solid_block(&bamboo_sapling), SolidClass::Passable);
        assert!(block_local_collision_boxes(&bamboo_sapling).is_empty());
    }

    #[test]
    fn test_vanilla_jar_selection_by_data_version() {
        use crate::anvil::WorldVersionInfo;
        use crate::assets::{read_jar_world_version, select_best_vanilla_jar};
        use std::io::Write;

        let tmp = std::env::temp_dir().join(format!("mcgm_jar_dv_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let make_jar = |fname: &str, world_version: i32| -> std::path::PathBuf {
            let p = tmp.join(fname);
            let f = std::fs::File::create(&p).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("assets/minecraft/textures/block/stone.png", opts).unwrap();
            zw.write_all(b"PNG").unwrap();
            zw.start_file("version.json", opts).unwrap();
            zw.write_all(format!(r#"{{"world_version": {}}}"#, world_version).as_bytes())
                .unwrap();
            zw.finish().unwrap();
            p
        };

        let jar_old = make_jar("1.20.1.jar", 3465);
        let jar_renamed = make_jar("custom_renamed_client.jar", 4556);
        assert_eq!(read_jar_world_version(&jar_old), Some(3465));
        assert_eq!(read_jar_world_version(&jar_renamed), Some(4556));

        let world_ver = WorldVersionInfo {
            name: Some("25w14a".to_string()),
            data_version: Some(4556),
        };
        let picked = select_best_vanilla_jar(
            vec![
                ("1.20.1".to_string(), jar_old),
                ("custom_renamed_client".to_string(), jar_renamed.clone()),
            ],
            Some(&world_ver),
        );
        assert_eq!(picked, Some(jar_renamed));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_water_vmt_omits_nocull_on_brushes_and_keeps_on_flow() {
        use crate::compiler::{compile_water_materials, ToolPaths};

        let tmp = std::env::temp_dir().join(format!("mcgm_water_vmt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&addon_dir).unwrap();

        let still_png = work_dir.join("mc_water.png");
        let flow_png = work_dir.join("mc_water_flow.png");
        crate::atlas::save_png_fast(&image::RgbaImage::new(16, 16), &still_png).unwrap();
        crate::atlas::save_png_fast(&image::RgbaImage::new(16, 16), &flow_png).unwrap();

        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            "@echo off\r\nfor %%F in (%2) do echo DUMMY > \"%~4\\%%~nF.vtf\"\r\n",
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 64,
        };

        compile_water_materials(&tools, "custom_mc", &still_png, &flow_png, &addon_dir, false).unwrap();

        let brush_vmt =
            std::fs::read_to_string(addon_dir.join("materials/custom_mc/mc_water.vmt")).unwrap();
        let beneath_vmt =
            std::fs::read_to_string(addon_dir.join("materials/custom_mc/mc_water_beneath.vmt"))
                .unwrap();
        let flow_vmt =
            std::fs::read_to_string(addon_dir.join("materials/models/custom_mc/mc_water_flow.vmt"))
                .unwrap();

        assert!(!brush_vmt.contains("$nocull"), "brush water VMT must not have $nocull");
        assert!(
            !beneath_vmt.contains("$nocull"),
            "beneath water VMT must not have $nocull"
        );
        assert!(
            flow_vmt.contains("\"$nocull\" \"1\""),
            "waterfall model VMT must keep $nocull"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_chunk_origin_non_occluder_chunk_stays_inside_bounds() {
        use crate::assets::{FaceDir, ModelElement, ModelFace, ResolvedBlockModel};
        use crate::atlas::{build_texture_atlas, SignAtlas};
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        let air = make_state("minecraft:air", &[]);
        let leaves = make_state("minecraft:oak_leaves", &[]);
        let mut faces = HashMap::new();
        faces.insert(
            FaceDir::Up,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/oak_leaves".to_string(),
                rotation: 0,
                tint_index: None,
                cullface: None,
            },
        );
        let leaves_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.0, 0.0, 0.0],
            to: [16.0, 16.0, 16.0],
            rotation: None,
            faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);

        // Vertical span 0..=64 with only one non-occluder block at y=0
        let coords = WorldCoords {
            min_x: 0,
            min_y: 0,
            min_z: 0,
            max_x: 0,
            max_y: 64,
            max_z: 0,
        };
        let mut blocks = vec![0u16; coords.size_y()];
        blocks[0] = 1;
        let grid = VoxelGrid {
            coords,
            palette: vec![air, leaves],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty, leaves_model];
        let (feat, reqs) = extract_features_and_requests(&grid, &models, [0.5, 0.0, 0.5], 39.37);
        let assets = AssetIndex::empty();
        let atlas = build_texture_atlas(&reqs, &assets, 4);
        let sign_atlas = SignAtlas {
            image: ImageBuffer::new(16, 16),
            rects: HashMap::new(),
        };
        let chunks = mesh_world_chunks_with_chunk_size(
            &grid,
            &models,
            &atlas,
            &sign_atlas,
            &feat,
            [0.5, 0.0, 0.5],
            39.37,
            16,
        );
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0].origin_source[2] < 5.0 * 39.37,
            "non-occluder chunk origin Z={} must stay near actual voxel instead of max_y + 1.5",
            chunks[0].origin_source[2]
        );
    }

    #[test]
    fn test_surface_prop_classification_and_chunk_collision_splitting() {
        let bs = |n: &str| BlockState::new(n.to_string(), BTreeMap::new());
        assert_eq!(classify_surface_prop(&bs("minecraft:stone")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:iron_ore")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:sandstone")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:mud_bricks")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:dirt")), SurfaceProp::Dirt);
        assert_eq!(classify_surface_prop(&bs("minecraft:grass_block")), SurfaceProp::Grass);
        assert_eq!(classify_surface_prop(&bs("minecraft:oak_leaves")), SurfaceProp::Grass);
        assert_eq!(classify_surface_prop(&bs("minecraft:oak_planks")), SurfaceProp::Wood);
        assert_eq!(classify_surface_prop(&bs("minecraft:oak_slab")), SurfaceProp::Wood);
        assert_eq!(classify_surface_prop(&bs("minecraft:stone_slab")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:iron_block")), SurfaceProp::Metal);
        assert_eq!(classify_surface_prop(&bs("minecraft:glass")), SurfaceProp::Glass);
        assert_eq!(classify_surface_prop(&bs("minecraft:sand")), SurfaceProp::Sand);
        assert_eq!(classify_surface_prop(&bs("minecraft:gravel")), SurfaceProp::Gravel);
        assert_eq!(classify_surface_prop(&bs("minecraft:snow_block")), SurfaceProp::Snow);
        assert_eq!(classify_surface_prop(&bs("minecraft:white_wool")), SurfaceProp::Carpet);

        let tmp = std::env::temp_dir().join(format!("mcgm_test_surf_split_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let cm = crate::mesher::ChunkMesh {
            cx: 0,
            cz: 0,
            name: "chunk_0_0".to_string(),
            origin_source: [0.0, 0.0, 0.0],
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [118.11, 39.37, 39.37],
            solid_tris: vec![MeshTri {
                material: "terrain_opaque".into(),
                pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                norm: [[0.0, 0.0, 1.0]; 3],
                uv: [[0.0, 0.0]; 3],
            }],
            translucent_tris: Vec::new(),
            has_cutout: false,
        };
        let boxes = vec![
            CollisionBox {
                box_coords: [0.0, 0.0, 0.0, 39.37, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            },
            CollisionBox {
                box_coords: [39.37, 0.0, 0.0, 78.74, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            },
            CollisionBox {
                box_coords: [78.74, 0.0, 0.0, 118.11, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Wood,
            },
        ];
        let art = emit_chunk_smds_and_qcs(&cm, &boxes, &tmp, "custom_mc", 64, "surf_map").unwrap();
        let main_qc = std::fs::read_to_string(&art.qc_path).unwrap();
        assert!(main_qc.contains("$surfaceprop \"rock\""), "main QC should use largest group (rock)");
        assert_eq!(art.extra_collision_models.len(), 1);
        let extra_qc = std::fs::read_to_string(&art.extra_collision_models[0].0).unwrap();
        assert!(extra_qc.contains("$surfaceprop \"wood\""), "extra collision QC should use wood");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_glua_dlight_chunk_fix_present() {
        let lua = include_str!("../lua/autorun/mcgm_physics.lua");
        assert!(lua.contains("mcgm_fix_dlight_chunks"));
        assert!(lua.contains("r_dynamic"));
        assert!(lua.contains("ApplyDlightChunkFix"));
        assert!(lua.contains("RestoreDlightSetting"));
    }

    #[test]
    fn test_gma_v3_native_packing_and_header() {
        let tmp = std::env::temp_dir().join(format!("mcgm_test_gma_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let addon_dir = tmp.join("addon");
        std::fs::create_dir_all(addon_dir.join("maps/thumb")).unwrap();
        std::fs::create_dir_all(addon_dir.join("lua/autorun")).unwrap();

        std::fs::write(addon_dir.join("maps/gm_mc_test.bsp"), b"VBSP\x14\x00\x00\x00testbsp").unwrap();
        std::fs::write(addon_dir.join("maps/gm_mc_test.vmf"), b"world {}").unwrap();
        std::fs::write(addon_dir.join("maps/thumb/gm_mc_test.png"), b"PNGDATA").unwrap();
        std::fs::write(addon_dir.join("lua/autorun/mcgm_physics.lua"), b"-- lua").unwrap();
        write_addon_metadata(&addon_dir, "gm_mc_test").unwrap();

        let out_gma = tmp.join("gm_mc_test.gma");
        pack_gma_v3_native(
            &addon_dir,
            &out_gma,
            "MCGM - gm_mc_test",
            r#"{"description":"test","type":"map","tags":["fun"]}"#,
        )
        .unwrap();

        let bytes = std::fs::read(&out_gma).unwrap();
        assert!(bytes.len() > 24);
        assert_eq!(&bytes[0..4], b"GMAD");
        assert_eq!(bytes[4], 3);
        let text_view = String::from_utf8_lossy(&bytes);
        assert!(text_view.contains("maps/gm_mc_test.bsp"));
        assert!(text_view.contains("maps/thumb/gm_mc_test.png"));
        assert!(text_view.contains("lua/autorun/mcgm_physics.lua"));
        assert!(!text_view.contains("maps/gm_mc_test.vmf"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_surface_prop_edge_cases_mossy_nylium_concrete_powder_muddy() {
        let bs = |n: &str| BlockState::new(n.to_string(), BTreeMap::new());
        assert_eq!(classify_surface_prop(&bs("minecraft:mossy_cobblestone")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:mossy_cobblestone_wall")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:mossy_stone_bricks")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:crimson_nylium")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:warped_nylium")), SurfaceProp::Rock);
        assert_eq!(classify_surface_prop(&bs("minecraft:muddy_mangrove_roots")), SurfaceProp::Dirt);
        assert_eq!(classify_surface_prop(&bs("minecraft:white_concrete_powder")), SurfaceProp::Sand);
        assert_eq!(classify_surface_prop(&bs("minecraft:moss_block")), SurfaceProp::Grass);
        assert_eq!(classify_surface_prop(&bs("minecraft:moss_carpet")), SurfaceProp::Grass);
        assert_eq!(classify_surface_prop(&bs("minecraft:smithing_table")), SurfaceProp::Wood);
    }

    #[test]
    fn test_vrad_ambient_for_lighting_styles() {
        assert_eq!(vrad_ambient_rgb_for_style("gmod"), None);
        assert_eq!(vrad_ambient_rgb_for_style("night"), Some(["0.015", "0.02", "0.035"]));
        assert_eq!(vrad_ambient_rgb_for_style("bright"), Some(["0.07", "0.075", "0.085"]));
        assert_eq!(vrad_ambient_rgb_for_style("minecraft"), Some(["0.04", "0.05", "0.06"]));
    }

    #[test]
    fn test_stage_and_cleanup_collision_vmts_when_no_import() {
        let tmp = std::env::temp_dir().join(format!("mcgm_test_vmt_stage_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let addon_dir = tmp.join("addon");
        let game_dir = tmp.join("game");
        let addon_cat = addon_dir.join("materials/custom_mc");
        let game_cat = game_dir.join("materials/custom_mc");
        std::fs::create_dir_all(&addon_cat).unwrap();
        std::fs::create_dir_all(&game_cat).unwrap();

        std::fs::write(addon_cat.join("col_rock.vmt"), "NEW_ROCK").unwrap();
        std::fs::write(addon_cat.join("col_wood.vmt"), "NEW_WOOD").unwrap();
        std::fs::write(game_cat.join("col_rock.vmt"), "PREEXISTING").unwrap();

        let staged = stage_collision_vmts_to_game_dir(&addon_dir, &game_dir, "custom_mc").unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0], game_cat.join("col_wood.vmt"));
        assert_eq!(std::fs::read_to_string(game_cat.join("col_rock.vmt")).unwrap(), "PREEXISTING");
        assert_eq!(std::fs::read_to_string(game_cat.join("col_wood.vmt")).unwrap(), "NEW_WOOD");

        cleanup_staged_files(&staged);
        assert!(!game_cat.join("col_wood.vmt").exists());
        assert_eq!(std::fs::read_to_string(game_cat.join("col_rock.vmt")).unwrap(), "PREEXISTING");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_cache_invalidated_when_qc_changes() {
        let tmp = std::env::temp_dir().join(format!("mcgm_test_qc_hash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let qc = tmp.join("chunk_0_0.qc");
        let phys = tmp.join("chunk_0_0_phys.smd");
        let addon_mdl = tmp.join("chunk_0_0.mdl");
        std::fs::write(&phys, "version 1\nend\n").unwrap();
        std::fs::write(
            &qc,
            "$modelname \"mcgm/chunk_0_0.mdl\"\n$surfaceprop \"dirt\"\n$collisionmodel \"chunk_0_0_phys.smd\" {\n}\n",
        )
        .unwrap();
        std::fs::write(&addon_mdl, b"IDST").unwrap();

        assert!(!qc_cache_signature_matches(&qc, &addon_mdl), "missing .mcgm_hash must invalidate legacy cache");
        write_qc_cache_signature(&qc, &addon_mdl);
        assert!(qc_cache_signature_matches(&qc, &addon_mdl));

        std::fs::write(
            &qc,
            "$modelname \"mcgm/chunk_0_0.mdl\"\n$surfaceprop \"rock\"\n$collisionmodel \"chunk_0_0_phys.smd\" {\n}\n",
        )
        .unwrap();
        assert!(!qc_cache_signature_matches(&qc, &addon_mdl), "changing QC surfaceprop must invalidate cache");

        write_qc_cache_signature(&qc, &addon_mdl);
        assert!(qc_cache_signature_matches(&qc, &addon_mdl));
        std::fs::write(&phys, "version 1\n// modified\nend\n").unwrap();
        assert!(!qc_cache_signature_matches(&qc, &addon_mdl), "changing phys SMD must invalidate cache");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_write_addon_metadata_ignores_vmf_hash_and_top_level_map_png() {
        let tmp = std::env::temp_dir().join(format!("mcgm_test_meta_ignore_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("maps/thumb")).unwrap();
        std::fs::create_dir_all(tmp.join("models/custom_mc")).unwrap();

        write_addon_metadata(&tmp, "gm_mc_demo").unwrap();
        let meta_txt = std::fs::read_to_string(tmp.join("addon.json")).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta_txt).unwrap();
        let ignores: Vec<&str> = meta["ignore"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(ignores.contains(&"*.vmf"));
        assert!(ignores.contains(&"*.mcgm_hash"));
        assert!(ignores.contains(&"maps/gm_mc_demo.png"));
        assert!(!ignores.contains(&"maps/*.png"));

        std::fs::write(tmp.join("maps/gm_mc_demo.bsp"), b"VBSP").unwrap();
        std::fs::write(tmp.join("maps/thumb/gm_mc_demo.png"), b"PNG").unwrap();
        std::fs::write(tmp.join("models/custom_mc/chunk.mcgm_hash"), b"deadbeef").unwrap();
        let out_gma = tmp.join("out.gma");
        pack_gma_v3_native(&tmp, &out_gma, "MCGM - gm_mc_demo", "{}").unwrap();
        let bytes = std::fs::read(&out_gma).unwrap();
        let text_view = String::from_utf8_lossy(&bytes);
        assert!(text_view.contains("maps/gm_mc_demo.bsp"));
        assert!(text_view.contains("maps/thumb/gm_mc_demo.png"));
        assert!(!text_view.contains("chunk.mcgm_hash"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_glua_dlight_fix_local_and_load_data() {
        let lua = include_str!("../lua/autorun/mcgm_physics.lua").replace("\r\n", "\n");
        assert!(lua.contains("local function ApplyDlightChunkFix()"));
        assert!(lua.contains("local function RestoreDlightSetting()"));
        assert!(!lua.contains("\nfunction ApplyDlightChunkFix()"));
        assert!(!lua.contains("\nfunction RestoreDlightSetting()"));
        let load_start = lua.find("local function LoadCurrentMapData()").expect("LoadCurrentMapData missing");
        let load_end = lua[load_start..].find("\nend\n").expect("LoadCurrentMapData end missing") + load_start;
        let load_body = &lua[load_start..load_end];
        assert!(load_body.contains("ApplyDlightChunkFix()"));
    }

    #[test]
    fn test_gui_completion_summary_localization_parity() {
        let summary_with_gma = crate::PipelineSummary {
            bsp_path: std::path::PathBuf::from("output/gm_mc.bsp"),
            gma_path: Some(std::path::PathBuf::from("output/gm_mc.gma")),
            gma_error: None,
            chunk_count: 12,
            door_count: 3,
            total_secs: 4.5,
            stage_timings: vec![],
        };
        let ru = format_done_summary(UiLang::Ru, &summary_with_gma);
        assert!(ru.contains("Готово") && ru.contains("GMA") && ru.contains("Чанк"));
        assert!(ru.contains("gm_mc.bsp") && ru.contains("gm_mc.gma"));
        let en = format_done_summary(UiLang::En, &summary_with_gma);
        assert!(en.contains("Done") && en.contains("GMA Addon") && en.contains("Chunks"));
        assert!(en.contains("gm_mc.bsp") && en.contains("gm_mc.gma"));

        let summary_no_gma = crate::PipelineSummary {
            gma_path: None,
            ..summary_with_gma
        };
        assert!(!format_done_summary(UiLang::Ru, &summary_no_gma).contains("GMA"));
        assert!(!format_done_summary(UiLang::En, &summary_no_gma).contains("GMA"));
    }

    #[test]
    fn test_gui_simplified_layout_modes_and_helpers() {
        let m_none = GuiLayoutMode {
            show_advanced: false,
            show_log: false,
        };
        let m_adv = GuiLayoutMode {
            show_advanced: true,
            show_log: false,
        };
        let m_log = GuiLayoutMode {
            show_advanced: false,
            show_log: true,
        };
        let m_both = GuiLayoutMode {
            show_advanced: true,
            show_log: true,
        };

        let def_none = compute_default_client_size(m_none);
        let def_adv = compute_default_client_size(m_adv);
        let def_log = compute_default_client_size(m_log);
        let def_both = compute_default_client_size(m_both);

        assert!(def_none.1 < 500 && def_none.0 <= 660);
        assert!(def_adv.1 > def_none.1 && def_adv.0 >= 660);
        assert!(def_log.1 > def_none.1);
        assert!(def_both.1 > def_adv.1 && def_both.1 > def_log.1);

        for mode in [m_none, m_adv, m_log, m_both] {
            let min_sz = compute_min_client_size(mode);
            let def_sz = compute_default_client_size(mode);
            assert!(min_sz.0 <= def_sz.0, "min_w <= def_w for {:?}", mode);
            assert!(min_sz.1 <= def_sz.1, "min_h <= def_h for {:?}", mode);
        }
        assert!(compute_min_client_size(m_adv).1 > compute_min_client_size(m_none).1);
        assert!(compute_min_client_size(m_log).1 > compute_min_client_size(m_none).1);

        assert_eq!(derive_model_name_from_map("gm_my_castle"), "my_castle");
        assert_eq!(derive_model_name_from_map("GM_Castle"), "castle");
        assert_eq!(derive_model_name_from_map("gm_"), "mc_export");
        assert_eq!(derive_model_name_from_map("My Build!"), "my_build");
        assert_eq!(derive_model_name_from_map("CON"), "mc_export");
        assert_eq!(derive_model_name_from_map("castle_1"), "castle_1");

        assert!(should_auto_replace_map_name("", None));
        assert!(should_auto_replace_map_name("gm_mc_custom", None));
        assert!(should_auto_replace_map_name("gm_mc_export", None));
        assert!(should_auto_replace_map_name("gm_minecraft", None));
        assert!(should_auto_replace_map_name("gm_world_a", Some("gm_world_a")));
        assert!(!should_auto_replace_map_name("gm_my_arena", Some("gm_world_a")));

        let discovered = vec![
            ("Alpha (saves)".to_string(), std::path::PathBuf::from(r"C:\saves\Alpha")),
            ("Beta (saves)".to_string(), std::path::PathBuf::from(r"C:\saves\Beta")),
        ];
        let (l1, idx1) = merge_discovered_world(
            &discovered,
            Some(std::path::Path::new(r"c:\saves\beta")),
            UiLang::En,
        );
        assert_eq!(l1.len(), 2);
        assert_eq!(idx1, Some(1));

        let (l2, idx2) = merge_discovered_world(
            &discovered,
            Some(std::path::Path::new(r"D:\Custom\MyWorld")),
            UiLang::Ru,
        );
        assert_eq!(l2.len(), 3);
        assert_eq!(idx2, Some(0));
        assert!(l2[0].0.contains("MyWorld"));

        let (l3, idx3) = merge_discovered_world(&discovered, None, UiLang::En);
        assert_eq!(l3.len(), 2);
        assert_eq!(idx3, Some(0));

        let ru_sel = format_selection_summary(UiLang::Ru, Some([-64, 32, -64, 64, 128, 64]));
        assert!(ru_sel.contains("129 × 97 × 129") && ru_sel.contains("блок"));
        assert!(!ru_sel.contains("blk") && !ru_sel.contains("чанков"));
        let en_sel = format_selection_summary(UiLang::En, Some([-64, 32, -64, 64, 128, 64]));
        assert!(en_sel.contains("129 × 97 × 129") && en_sel.contains("block"));
        assert!(!en_sel.contains("blk") && !en_sel.contains("chunks"));
        assert_eq!(format_selection_summary(UiLang::Ru, None), tr_text(UiLang::Ru, "sel_empty"));
        assert_eq!(format_selection_summary(UiLang::En, None), tr_text(UiLang::En, "sel_empty"));

        let keys = [
            "ui_title",
            "step_world",
            "step_area",
            "step_map",
            "rad_small",
            "rad_medium",
            "rad_large",
            "btn_adv_show",
            "btn_adv_hide",
            "btn_log_show",
            "btn_log_hide",
            "grp_adv",
            "lbl_log_hdr",
            "world_custom",
            "btn_start",
            "btn_running",
            "btn_open_out",
            "btn_launch_gmod",
            "preset_preview",
            "preset_balanced",
            "preset_hq",
            "preset_custom",
            "light_minecraft",
            "light_gmod",
            "light_bright",
            "light_night",
        ];
        for key in keys {
            assert!(!tr_text(UiLang::Ru, key).is_empty(), "Missing RU key: {}", key);
            assert!(!tr_text(UiLang::En, key).is_empty(), "Missing EN key: {}", key);
        }
    }

    #[test]
    fn test_gui_simple_mode_coords_and_compact_titles() {
        assert_eq!(tr_text(UiLang::Ru, "win_title"), "MCGM");
        assert_eq!(tr_text(UiLang::En, "win_title"), "MCGM");
        assert_eq!(tr_text(UiLang::Ru, "ui_title"), "Конвертер");
        assert_eq!(tr_text(UiLang::En, "ui_title"), "Converter");

        for coord_id in [
            ID_LBL_COORDS_MIN,
            ID_EDIT_COORD_X1,
            ID_EDIT_COORD_Y1,
            ID_EDIT_COORD_Z1,
            ID_LBL_COORDS_MAX,
            ID_EDIT_COORD_X2,
            ID_EDIT_COORD_Y2,
            ID_EDIT_COORD_Z2,
        ] {
            assert!(
                !is_advanced_only_control(coord_id),
                "Coordinate control {} must remain visible in Simple mode",
                coord_id
            );
        }

        for adv_id in [
            ID_GRP_ADV,
            ID_EDIT_WORLD,
            ID_COMBO_DIM,
            ID_EDIT_SCALE,
            ID_EDIT_NAME,
            ID_EDIT_CAT,
            ID_CHK_EXPORT_VMF,
            ID_CHK_VMF_BRUSHES,
        ] {
            assert!(
                is_advanced_only_control(adv_id),
                "Control {} should be in Advanced panel",
                adv_id
            );
        }
        for out_id in [crate::gui::ID_LBL_OUTDIR, ID_EDIT_OUTDIR, crate::gui::ID_BTN_BROWSE_OUT] {
            assert!(
                !is_advanced_only_control(out_id),
                "Output directory control {} must be visible in Simple mode",
                out_id
            );
        }
    }

    fn make_temp_test_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mcgm_{}_{}_{}", tag, std::process::id(), nanos));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn make_dummy_world(saves_dir: &std::path::Path, world_name: &str) -> std::path::PathBuf {
        let w = saves_dir.join(world_name);
        std::fs::create_dir_all(&w).expect("create world dir");
        std::fs::write(w.join("level.dat"), b"dummy").expect("write level.dat");
        w
    }

    #[test]
    fn test_instances_vanilla_profiles_and_version_isolation() {
        let root = make_temp_test_dir("vanilla_inst");
        let mc_dir = root.join(".minecraft");
        std::fs::create_dir_all(&mc_dir).unwrap();

        // 1. Default .minecraft/saves
        make_dummy_world(&mc_dir.join("saves"), "DefaultWorld");

        // 2. Absolute custom gameDir
        let custom_dir = root.join("custom_1_20_1");
        make_dummy_world(&custom_dir.join("saves"), "ModdedBase");

        // 3. Relative gameDir inside .minecraft
        make_dummy_world(&mc_dir.join("instances").join("rel_pack").join("saves"), "RelWorld");

        // 4. Microsoft Store profile
        let ms_dir = root.join("ms_profile");
        make_dummy_world(&ms_dir.join("saves"), "StoreWorld");

        // 5. Version-isolated saves without a profile entry
        make_dummy_world(&mc_dir.join("versions").join("1.21.4").join("saves"), "IsolatedWorld");

        let profiles_json = serde_json::json!({
            "profiles": {
                "fabric_pack": {
                    "name": "Fabric 1.20.1",
                    "lastVersionId": "fabric-loader-0.15.11-1.20.1",
                    "gameDir": custom_dir.to_string_lossy()
                },
                "rel_profile": {
                    "name": "",
                    "lastVersionId": "1.21.1",
                    "gameDir": "instances/rel_pack"
                },
                "latest_release": {
                    "name": "Latest Release",
                    "lastVersionId": "latest-release"
                }
            }
        });
        std::fs::write(
            mc_dir.join("launcher_profiles.json"),
            serde_json::to_string_pretty(&profiles_json).unwrap(),
        )
        .unwrap();

        let ms_profiles_json = serde_json::json!({
            "profiles": {
                "ms_pack": {
                    "name": "MS Store Survival",
                    "lastVersionId": "1.21.3",
                    "gameDir": ms_dir.to_string_lossy()
                }
            }
        });
        std::fs::write(
            mc_dir.join("launcher_profiles_microsoft_store.json"),
            serde_json::to_string_pretty(&ms_profiles_json).unwrap(),
        )
        .unwrap();

        let instances = discover_vanilla_instances(&mc_dir);
        assert_eq!(instances.len(), 5, "Expected 5 distinct vanilla instances, got: {:?}", instances);
        assert!(instances.iter().all(|i| i.launcher == LauncherKind::Vanilla));

        let mut seen_saves = HashSet::new();
        for inst in &instances {
            let key = std::fs::canonicalize(&inst.saves_dir)
                .unwrap_or_else(|_| inst.saves_dir.clone())
                .to_string_lossy()
                .to_lowercase();
            assert!(seen_saves.insert(key), "Duplicate saves_dir: {}", inst.saves_dir.display());
        }

        let fabric = instances.iter().find(|i| i.name == "Fabric 1.20.1").expect("Fabric 1.20.1 missing");
        assert_eq!(fabric.mc_version.as_deref(), Some("1.20.1"));
        assert_eq!(fabric.loader.as_deref(), Some("Fabric"));
        assert_eq!(fabric.worlds.len(), 1);
        assert_eq!(fabric.worlds[0].name, "ModdedBase");

        let rel = instances.iter().find(|i| i.worlds.iter().any(|w| w.name == "RelWorld")).expect("RelWorld instance missing");
        assert_eq!(rel.mc_version.as_deref(), Some("1.21.1"));

        let ms = instances.iter().find(|i| i.name == "MS Store Survival").expect("MS Store instance missing");
        assert_eq!(ms.mc_version.as_deref(), Some("1.21.3"));
        assert_eq!(ms.worlds[0].name, "StoreWorld");

        let iso = instances.iter().find(|i| i.worlds.iter().any(|w| w.name == "IsolatedWorld")).expect("IsolatedWorld missing");
        assert_eq!(iso.name, "1.21.4");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_instances_prism_and_multimc_cfg_and_metadata() {
        let ini_sample = "\u{feff}[OtherSection]\nInstanceDir=wrong_dir\n[General]\n# comment\nInstanceDir=\"C:\\\\Games\\\\Prism\\\\Instances\"\nName=Test = Value\nEmpty=\nVariant=@Variant(\\0\\0)\n";
        assert_eq!(
            parse_ini_value(ini_sample, "instancedir").as_deref(),
            Some(r"C:\Games\Prism\Instances")
        );
        assert_eq!(parse_ini_value(ini_sample, "name").as_deref(), Some("Test = Value"));
        assert_eq!(parse_ini_value(ini_sample, "empty"), None);
        assert_eq!(parse_ini_value(ini_sample, "variant"), None);

        let top_level_ini = "name=TopLevel\n";
        assert_eq!(parse_ini_value(top_level_ini, "name").as_deref(), Some("TopLevel"));

        let verbatim = std::path::Path::new(r"\\?\C:\Games\PrismLauncher\instances");
        assert_eq!(
            strip_windows_verbatim_prefix(verbatim),
            std::path::PathBuf::from(r"C:\Games\PrismLauncher\instances")
        );
        let unc_verbatim = std::path::Path::new(r"\\?\UNC\server\share\saves");
        assert_eq!(
            strip_windows_verbatim_prefix(unc_verbatim),
            std::path::PathBuf::from(r"\\server\share\saves")
        );

        let root = make_temp_test_dir("prism_mmc");
        let prism_root = root.join("PrismLauncher");
        let prism_custom_inst = root.join("PrismCustomInstances");
        std::fs::create_dir_all(&prism_root).unwrap();
        std::fs::create_dir_all(&prism_custom_inst).unwrap();

        let cfg_text = format!(
            "[General]\nInstanceDir={}\n",
            prism_custom_inst.to_string_lossy().replace('\\', "/")
        );
        std::fs::write(prism_root.join("prismlauncher.cfg"), cfg_text).unwrap();

        // create_pack with instance.cfg + mmc-pack.json + minecraft/saves
        let create_dir = prism_custom_inst.join("create_pack");
        make_dummy_world(&create_dir.join("minecraft").join("saves"), "Factory");
        std::fs::write(create_dir.join("instance.cfg"), "[General]\nname=Create Modpack\n").unwrap();
        let pack_json = serde_json::json!({
            "components": [
                {"uid": "net.minecraft", "version": "1.20.1"},
                {"uid": "net.fabricmc.fabric-loader", "version": "0.15.11"}
            ]
        });
        std::fs::write(create_dir.join("mmc-pack.json"), pack_json.to_string()).unwrap();

        // legacy_inst without instance.cfg, using .minecraft/saves
        let legacy_dir = prism_custom_inst.join("legacy_inst");
        make_dummy_world(&legacy_dir.join(".minecraft").join("saves"), "OldWorld");

        // Ignored temp dirs
        make_dummy_world(&prism_custom_inst.join("_MMC_TEMP").join("minecraft").join("saves"), "Ignored");
        make_dummy_world(&prism_custom_inst.join(".LAUNCHER_TEMP").join("minecraft").join("saves"), "Ignored2");

        let prism_instances = discover_multimc_family_instances(&prism_root, LauncherKind::PrismLauncher);
        assert_eq!(prism_instances.len(), 2, "Expected 2 prism instances, got: {:?}", prism_instances);
        let create_inst = prism_instances.iter().find(|i| i.name == "Create Modpack").expect("Create Modpack missing");
        assert_eq!(create_inst.launcher, LauncherKind::PrismLauncher);
        assert_eq!(create_inst.mc_version.as_deref(), Some("1.20.1"));
        assert_eq!(create_inst.loader.as_deref(), Some("Fabric"));
        assert_eq!(create_inst.worlds[0].name, "Factory");
        assert!(create_inst.display_label().contains("Prism"));
        assert!(create_inst.display_label().contains("1.20.1"));

        let legacy = prism_instances.iter().find(|i| i.name == "legacy_inst").expect("legacy_inst missing");
        assert_eq!(legacy.worlds[0].name, "OldWorld");

        // MultiMC root
        let mmc_root = root.join("MultiMC");
        std::fs::create_dir_all(&mmc_root).unwrap();
        std::fs::write(mmc_root.join("multimc.cfg"), "InstanceDir=instances\n").unwrap();
        let sky_dir = mmc_root.join("instances").join("skyblock");
        make_dummy_world(&sky_dir.join(".minecraft").join("saves"), "Island");
        std::fs::write(sky_dir.join("instance.cfg"), "name=\"Skyblock Pack\"\n").unwrap();
        let mmc_pack = serde_json::json!({
            "components": [
                {"uid": "net.minecraft", "version": "1.19.4"},
                {"uid": "net.minecraftforge", "version": "45.2.0"}
            ]
        });
        std::fs::write(sky_dir.join("mmc-pack.json"), mmc_pack.to_string()).unwrap();

        let (v, l) = parse_mmc_pack_metadata(&mmc_pack.to_string());
        assert_eq!(v.as_deref(), Some("1.19.4"));
        assert_eq!(l.as_deref(), Some("Forge"));

        let mmc_instances = discover_multimc_family_instances(&mmc_root, LauncherKind::MultiMC);
        assert_eq!(mmc_instances.len(), 1);
        assert_eq!(mmc_instances[0].launcher, LauncherKind::MultiMC);
        assert_eq!(mmc_instances[0].name, "Skyblock Pack");
        assert_eq!(mmc_instances[0].mc_version.as_deref(), Some("1.19.4"));
        assert_eq!(mmc_instances[0].loader.as_deref(), Some("Forge"));
        assert_eq!(mmc_instances[0].worlds[0].name, "Island");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_instances_custom_dir_variants_and_query_matching() {
        let root = make_temp_test_dir("custom_variants");

        // (a) Prism root
        let prism_root = root.join("PrismRoot");
        std::fs::create_dir_all(&prism_root).unwrap();
        std::fs::write(prism_root.join("prismlauncher.cfg"), "[General]\nInstanceDir=instances\n").unwrap();
        let inst_a = prism_root.join("instances").join("pack_alpha");
        make_dummy_world(&inst_a.join("minecraft").join("saves"), "AlphaWorld");
        std::fs::write(inst_a.join("instance.cfg"), "name=Alpha Pack\n").unwrap();
        assert_eq!(scan_custom_dir(&prism_root).len(), 1);

        // (b) instances/ dir directly
        assert_eq!(scan_custom_dir(&prism_root.join("instances")).len(), 1);

        // (c) single instance dir
        let single_inst = scan_custom_dir(&inst_a);
        assert_eq!(single_inst.len(), 1);
        assert_eq!(single_inst[0].name, "Alpha Pack");

        // (d) plain saves/ dir
        let plain_saves = root.join("PlainSaves");
        let w1 = make_dummy_world(&plain_saves, "WorldOne");
        let plain_res = scan_custom_dir(&plain_saves);
        assert_eq!(plain_res.len(), 1);
        assert_eq!(plain_res[0].saves_dir, plain_saves);
        assert_eq!(plain_res[0].worlds.len(), 1);

        // (e) single world dir
        let single_world_res = scan_custom_dir(&w1);
        assert_eq!(single_world_res.len(), 1);
        assert_eq!(single_world_res[0].worlds[0].name, "WorldOne");

        // Query matching: exact match preferred over substring, plus folder name match
        let list = vec![
            MinecraftInstance {
                launcher: LauncherKind::PrismLauncher,
                name: "Create Modpack".to_string(),
                mc_version: Some("1.20.1".to_string()),
                loader: Some("Fabric".to_string()),
                instance_dir: root.join("instances").join("create_folder"),
                saves_dir: root.join("instances").join("create_folder").join("minecraft").join("saves"),
                worlds: vec![
                    DiscoveredWorld {
                        name: "Newest".to_string(),
                        path: root.join("Newest"),
                        modified_secs: 200,
                    },
                    DiscoveredWorld {
                        name: "Older".to_string(),
                        path: root.join("Older"),
                        modified_secs: 100,
                    },
                ],
            },
            MinecraftInstance {
                launcher: LauncherKind::MultiMC,
                name: "Create".to_string(),
                mc_version: Some("1.19.2".to_string()),
                loader: None,
                instance_dir: root.join("instances").join("create_exact"),
                saves_dir: root.join("instances").join("create_exact").join("minecraft").join("saves"),
                worlds: vec![],
            },
        ];

        let exact = find_instance_by_query(&list, "create").expect("exact match");
        assert_eq!(exact.name, "Create");

        let by_folder = find_instance_by_query(&list, "create_folder").expect("folder match");
        assert_eq!(by_folder.name, "Create Modpack");

        assert_eq!(
            resolve_world_in_instance(&list[0], None),
            Some(list[0].worlds[0].path.clone())
        );
        assert_eq!(
            resolve_world_in_instance(&list[0], Some(std::path::Path::new("older"))),
            Some(list[0].worlds[1].path.clone())
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_gui_instance_helpers_and_localization_parity() {
        let instances = vec![
            MinecraftInstance {
                launcher: LauncherKind::Vanilla,
                name: "Default (.minecraft)".to_string(),
                mc_version: None,
                loader: None,
                instance_dir: std::path::PathBuf::from(r"C:\mc\.minecraft"),
                saves_dir: std::path::PathBuf::from(r"C:\mc\.minecraft\saves"),
                worlds: vec![DiscoveredWorld {
                    name: "Survival".to_string(),
                    path: std::path::PathBuf::from(r"C:\mc\.minecraft\saves\Survival"),
                    modified_secs: 10,
                }],
            },
            MinecraftInstance {
                launcher: LauncherKind::PrismLauncher,
                name: "Create Modpack".to_string(),
                mc_version: Some("1.20.1".to_string()),
                loader: Some("Fabric".to_string()),
                instance_dir: std::path::PathBuf::from(r"D:\Prism\instances\Create"),
                saves_dir: std::path::PathBuf::from(r"D:\Prism\instances\Create\minecraft\saves"),
                worlds: vec![DiscoveredWorld {
                    name: "Factory".to_string(),
                    path: std::path::PathBuf::from(r"D:\Prism\instances\Create\minecraft\saves\Factory"),
                    modified_secs: 20,
                }],
            },
        ];

        let items_en = build_instance_combo_items(&instances, UiLang::En);
        assert_eq!(items_en.len(), 3);
        assert_eq!(items_en[0], "All instances (2)");
        assert!(items_en[1].contains("Vanilla"));
        assert!(items_en[2].contains("Prism") && items_en[2].contains("Create Modpack"));

        let items_ru = build_instance_combo_items(&instances, UiLang::Ru);
        assert_eq!(items_ru[0], "Все инстансы (2)");

        let all_worlds = worlds_for_instance_selection(&instances, None);
        assert_eq!(all_worlds.len(), 2);
        assert!(all_worlds[0].0.contains("Survival") && all_worlds[0].0.contains("Vanilla"));
        assert!(all_worlds[1].0.contains("Factory") && all_worlds[1].0.contains("Prism: Create Modpack"));

        let prism_worlds = worlds_for_instance_selection(&instances, Some(1));
        assert_eq!(prism_worlds.len(), 1);
        assert_eq!(prism_worlds[0].0, "Factory");

        assert_eq!(
            resolve_browse_initial_dir(&instances, Some(1), None),
            Some(std::path::PathBuf::from(r"D:\Prism\instances\Create\minecraft\saves"))
        );
        assert_eq!(
            resolve_browse_initial_dir(
                &instances,
                None,
                Some(std::path::Path::new(r"E:\Custom\saves\MyWorld"))
            ),
            Some(std::path::PathBuf::from(r"E:\Custom\saves"))
        );
        assert_eq!(
            resolve_browse_initial_dir(&instances, None, None),
            Some(std::path::PathBuf::from(r"C:\mc\.minecraft\saves"))
        );

        for key in [
            "lbl_instance",
            "lbl_world_row",
            "btn_add_instance",
            "instances_all",
            "browse_instance",
            "worlds_none_in_instance",
            "instances_added_none",
        ] {
            assert!(!tr_text(UiLang::Ru, key).is_empty(), "Missing RU key: {}", key);
            assert!(!tr_text(UiLang::En, key).is_empty(), "Missing EN key: {}", key);
        }

        let cli_list = CliArgs::try_parse_from(["mcgm", "--list-instances"]).unwrap();
        assert!(cli_list.list_instances);
        assert!(!cli_list.should_launch_gui(2));

        let cli_inst = CliArgs::try_parse_from(["mcgm", "--instance", "Create"]).unwrap();
        assert_eq!(cli_inst.instance.as_deref(), Some("Create"));
    }

    #[test]
    fn test_gui_modern_theme_dpi_cards_and_manifest() {
        let manifest_path = std::path::Path::new("app.manifest");
        assert!(manifest_path.is_file(), "app.manifest must exist at repo root");
        let manifest = std::fs::read_to_string(manifest_path).expect("read app.manifest");
        assert!(manifest.contains("Microsoft.Windows.Common-Controls"));
        assert!(manifest.contains("6.0.0.0"));
        assert!(manifest.contains("PerMonitorV2"));

        let pal = light_palette();
        let assert_achromatic = |name: &str, c: u32| {
            let r = c & 0xFF;
            let g = (c >> 8) & 0xFF;
            let b = (c >> 16) & 0xFF;
            assert!(
                r == g && g == b,
                "Palette color {} must be neutral grayscale (r==g==b), got ({}, {}, {})",
                name,
                r,
                g,
                b
            );
        };
        for (name, col) in [
            ("canvas_bg", pal.canvas_bg),
            ("card_bg", pal.card_bg),
            ("card_border", pal.card_border),
            ("input_bg", pal.input_bg),
            ("input_border", pal.input_border),
            ("log_bg", pal.log_bg),
            ("log_text", pal.log_text),
            ("text_primary", pal.text_primary),
            ("text_secondary", pal.text_secondary),
            ("text_muted", pal.text_muted),
            ("accent_text", pal.accent_text),
            ("primary_bg", pal.primary_bg),
            ("primary_hover", pal.primary_hover),
            ("primary_pressed", pal.primary_pressed),
            ("primary_disabled", pal.primary_disabled),
            ("primary_text", pal.primary_text),
            ("danger_bg", pal.danger_bg),
            ("danger_hover", pal.danger_hover),
            ("danger_pressed", pal.danger_pressed),
            ("btn_bg", pal.btn_bg),
            ("btn_hover", pal.btn_hover),
            ("btn_pressed", pal.btn_pressed),
            ("btn_border", pal.btn_border),
            ("btn_text", pal.btn_text),
            ("pill_bg", pal.pill_bg),
            ("pill_hover", pal.pill_hover),
            ("pill_border", pal.pill_border),
            ("chk_accent", pal.chk_accent),
            ("chk_border", pal.chk_border),
            ("chk_hover_border", pal.chk_hover_border),
            ("prog_track", pal.prog_track),
            ("prog_fill", pal.prog_fill),
        ] {
            assert_achromatic(name, col);
        }

        let luma = |c: u32| -> u32 {
            let r = c & 0xFF;
            let g = (c >> 8) & 0xFF;
            let b = (c >> 16) & 0xFF;
            (r * 299 + g * 587 + b * 114) / 1000
        };
        assert!(luma(pal.canvas_bg) > 220);
        assert_eq!(luma(pal.card_bg), 255);
        assert!(luma(pal.text_primary) < 50);

        assert_ne!(GUI_BOLD_FONT_FACE, GUI_REGULAR_FONT_FACE);
        assert_eq!(GUI_BOLD_FONT_FACE, "Bahnschrift SemiBold");

        assert_eq!(scale_px(100, 96), 100);
        assert_eq!(scale_px(100, 144), 150);
        assert_eq!(scale_px(100, 192), 200);
        assert_eq!(scale_px(100, 0), 100);

        assert_eq!(classify_button_kind(ID_BTN_START), ControlVisualKind::PrimaryButton);
        assert_eq!(classify_button_kind(ID_BTN_STOP), ControlVisualKind::DangerButton);
        assert_eq!(classify_button_kind(ID_BTN_RAD_32), ControlVisualKind::PillButton);
        assert_eq!(classify_button_kind(ID_BTN_RAD_64), ControlVisualKind::PillButton);
        assert_eq!(classify_button_kind(ID_BTN_RAD_128), ControlVisualKind::PillButton);
        assert_eq!(classify_button_kind(ID_CHK_IMPORT_GMOD), ControlVisualKind::Checkbox);
        assert_eq!(classify_button_kind(ID_CHK_MC_PHYSICS), ControlVisualKind::Checkbox);
        assert_eq!(classify_button_kind(ID_BTN_BROWSE_WORLD), ControlVisualKind::SecondaryButton);

        let simple_cards = compute_card_rects(640, GuiLayoutMode::default(), 96);
        assert_eq!(simple_cards.len(), 3);
        let adv_cards = compute_card_rects(
            680,
            GuiLayoutMode {
                show_advanced: true,
                show_log: false,
            },
            96,
        );
        assert_eq!(adv_cards.len(), 4);

        for cards in [&simple_cards, &adv_cards] {
            for (idx, &(l, t, r, b)) in cards.iter().enumerate() {
                assert!(l < r && t < b, "card {} must have positive dimensions", idx);
                assert!(r <= 680, "card {} right edge {} exceeds client width", idx, r);
                if idx > 0 {
                    assert!(
                        cards[idx - 1].3 < t,
                        "card {} overlaps previous card: prev_bottom={} cur_top={}",
                        idx,
                        cards[idx - 1].3,
                        t
                    );
                }
            }
        }

        let def = compute_default_client_size(GuiLayoutMode::default());
        assert!(def.1 < 500 && def.0 <= 660);
    }

    #[test]
    fn test_mc_26_2_palette_formats_and_players_data_spawn() {
        fn push_nbt_str(buf: &mut Vec<u8>, s: &str) {
            let b = s.as_bytes();
            buf.extend_from_slice(&(b.len() as u16).to_be_bytes());
            buf.extend_from_slice(b);
        }
        fn push_tag_header(buf: &mut Vec<u8>, tag: u8, name: &str) {
            buf.push(tag);
            push_nbt_str(buf, name);
        }
        fn push_int(buf: &mut Vec<u8>, name: &str, v: i32) {
            push_tag_header(buf, 3, name);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        fn push_byte(buf: &mut Vec<u8>, name: &str, v: i8) {
            push_tag_header(buf, 1, name);
            buf.push(v as u8);
        }
        fn push_str_tag(buf: &mut Vec<u8>, name: &str, val: &str) {
            push_tag_header(buf, 8, name);
            push_nbt_str(buf, val);
        }

        let root = make_temp_test_dir("mc_26_2");
        let region_dir = root
            .join("dimensions")
            .join("minecraft")
            .join("overworld")
            .join("region");
        std::fs::create_dir_all(&region_dir).unwrap();

        // level.dat with Data.Version = 26.2, DataVersion = 5023, Data.spawn.pos = [0, 64, 0]
        let mut level_nbt = Vec::new();
        push_tag_header(&mut level_nbt, 10, "");
        push_tag_header(&mut level_nbt, 10, "Data");
        push_int(&mut level_nbt, "DataVersion", 5023);
        push_tag_header(&mut level_nbt, 10, "Version");
        push_str_tag(&mut level_nbt, "Name", "26.2");
        push_int(&mut level_nbt, "Id", 5023);
        level_nbt.push(0); // end Version
        push_tag_header(&mut level_nbt, 10, "spawn");
        push_tag_header(&mut level_nbt, 11, "pos");
        level_nbt.extend_from_slice(&3i32.to_be_bytes());
        for v in [0i32, 64, 0] {
            level_nbt.extend_from_slice(&v.to_be_bytes());
        }
        level_nbt.push(0); // end spawn
        level_nbt.push(0); // end Data
        level_nbt.push(0); // end root
        let mut gz_level = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        gz_level.write_all(&level_nbt).unwrap();
        std::fs::write(root.join("level.dat"), gz_level.finish().unwrap()).unwrap();

        // players/data/._mac_junk.dat (must be skipped) + players/data/player1.dat
        let pdata = root.join("players").join("data");
        std::fs::create_dir_all(&pdata).unwrap();
        std::fs::write(pdata.join("._mac_junk.dat"), b"AppleDoubleGarbage").unwrap();
        let mut player_nbt = Vec::new();
        push_tag_header(&mut player_nbt, 10, "");
        push_tag_header(&mut player_nbt, 9, "Pos");
        player_nbt.push(6); // TAG_Double
        player_nbt.extend_from_slice(&3i32.to_be_bytes());
        for d in [12.5f64, 70.0, -8.5] {
            player_nbt.extend_from_slice(&d.to_be_bytes());
        }
        player_nbt.push(0);
        let mut gz_player = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        gz_player.write_all(&player_nbt).unwrap();
        std::fs::write(pdata.join("player1.dat"), gz_player.finish().unwrap()).unwrap();

        // Chunk (0, 0) with DataVersion 5023 and 2 sections:
        // Y = 3: uniform TAG_List<TAG_String> ["minecraft:stone"]
        // Y = 4: heterogeneous TAG_List<TAG_Compound> with {"": ...}, {"id": ..., "properties": ...}, {"Name": ..., "Properties": ...}
        let mut chunk_nbt = Vec::new();
        push_tag_header(&mut chunk_nbt, 10, "");
        push_int(&mut chunk_nbt, "DataVersion", 5023);
        push_int(&mut chunk_nbt, "xPos", 0);
        push_int(&mut chunk_nbt, "zPos", 0);
        push_tag_header(&mut chunk_nbt, 9, "sections");
        chunk_nbt.push(10);
        chunk_nbt.extend_from_slice(&2i32.to_be_bytes());

        // Section Y=3 (String palette)
        push_byte(&mut chunk_nbt, "Y", 3);
        push_tag_header(&mut chunk_nbt, 10, "block_states");
        push_tag_header(&mut chunk_nbt, 9, "palette");
        chunk_nbt.push(8); // TAG_String list!
        chunk_nbt.extend_from_slice(&1i32.to_be_bytes());
        push_nbt_str(&mut chunk_nbt, "minecraft:stone");
        chunk_nbt.push(0); // end block_states
        chunk_nbt.push(0); // end section Y=3

        // Section Y=4 (Mixed 26.2 compound palette)
        push_byte(&mut chunk_nbt, "Y", 4);
        push_tag_header(&mut chunk_nbt, 10, "block_states");
        push_tag_header(&mut chunk_nbt, 9, "palette");
        chunk_nbt.push(10); // TAG_Compound
        chunk_nbt.extend_from_slice(&3i32.to_be_bytes());
        // [0]: {"": "minecraft:dirt"}
        push_str_tag(&mut chunk_nbt, "", "minecraft:dirt");
        chunk_nbt.push(0);
        // [1]: {"id": "minecraft:oak_leaves", "properties": {"distance": "1", "persistent": "true"}}
        push_str_tag(&mut chunk_nbt, "id", "minecraft:oak_leaves");
        push_tag_header(&mut chunk_nbt, 10, "properties");
        push_str_tag(&mut chunk_nbt, "distance", "1");
        push_str_tag(&mut chunk_nbt, "persistent", "true");
        chunk_nbt.push(0);
        chunk_nbt.push(0);
        // [2]: {"Name": "minecraft:grass_block", "Properties": {"snowy": "false"}}
        push_str_tag(&mut chunk_nbt, "Name", "minecraft:grass_block");
        push_tag_header(&mut chunk_nbt, 10, "Properties");
        push_str_tag(&mut chunk_nbt, "snowy", "false");
        chunk_nbt.push(0);
        chunk_nbt.push(0);

        // data: index 0 -> 0 (dirt), index 1 -> 1 (oak_leaves), index 2 -> 2 (grass_block)
        let mut longs = vec![0i64; 256];
        longs[0] = (1i64 << 4) | (2i64 << 8);
        push_tag_header(&mut chunk_nbt, 12, "data");
        chunk_nbt.extend_from_slice(&(longs.len() as i32).to_be_bytes());
        for l in longs {
            chunk_nbt.extend_from_slice(&l.to_be_bytes());
        }
        chunk_nbt.push(0); // end block_states

        // biomes: ["minecraft:forest"]
        push_tag_header(&mut chunk_nbt, 10, "biomes");
        push_tag_header(&mut chunk_nbt, 9, "palette");
        chunk_nbt.push(8);
        chunk_nbt.extend_from_slice(&1i32.to_be_bytes());
        push_nbt_str(&mut chunk_nbt, "minecraft:forest");
        chunk_nbt.push(0); // end biomes
        chunk_nbt.push(0); // end section Y=4
        chunk_nbt.push(0); // end root

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&chunk_nbt).unwrap();
        let compressed = enc.finish().unwrap();

        let mut mca = vec![0u8; 4096 * 3];
        mca[0..4].copy_from_slice(&((2u32 << 8) | 1u32).to_be_bytes());
        let p_off = 4096 * 2;
        let payload_len = (compressed.len() + 1) as u32;
        mca[p_off..p_off + 4].copy_from_slice(&payload_len.to_be_bytes());
        mca[p_off + 4] = 2; // zlib
        mca[p_off + 5..p_off + 5 + compressed.len()].copy_from_slice(&compressed);
        std::fs::write(region_dir.join("r.0.0.mca"), mca).unwrap();

        assert_eq!(
            crate::anvil::read_level_spawn(&root),
            Some([12.5, 70.0, -8.5])
        );

        let coords = WorldCoords {
            min_x: 0,
            min_y: 63,
            min_z: 0,
            max_x: 3,
            max_y: 64,
            max_z: 0,
        };
        let grid = load_world_region(&root, "overworld", coords, 39.37)
            .expect("load_world_region must parse MC 26.2 string & id/properties palettes");
        assert_eq!(grid.get_block(0, 63, 0).name, "minecraft:stone");
        assert_eq!(grid.get_block(0, 64, 0).name, "minecraft:dirt");
        let leaves = grid.get_block(1, 64, 0);
        assert_eq!(leaves.name, "minecraft:oak_leaves");
        assert_eq!(leaves.prop("distance"), Some("1"));
        assert_eq!(leaves.prop("persistent"), Some("true"));
        let grass = grid.get_block(2, 64, 0);
        assert_eq!(grass.name, "minecraft:grass_block");
        assert_eq!(grass.prop("snowy"), Some("false"));

        let discovered = scan_custom_dir(&root);
        assert_eq!(discovered.len(), 1, "scan_custom_dir must recognize MC 26.2 dimensions/minecraft/overworld/region world");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_gui_simplified_concise_labels() {
        assert_eq!(tr_text(UiLang::Ru, "btn_start"), "Создать карту");
        assert_eq!(tr_text(UiLang::En, "btn_start"), "Create map");
        assert_eq!(tr_text(UiLang::Ru, "step_world"), "1. Мир");
        assert_eq!(tr_text(UiLang::Ru, "rad_small"), "±32");
        assert_eq!(tr_text(UiLang::Ru, "status_ready"), "Готово");
    }

    #[test]
    fn test_vvis_prt_auto_fast_guard() {
        use crate::compiler::{
            parse_prt_portal_counts, should_use_fast_vvis, VVIS_AUTO_FAST_MAX_CLUSTERS,
            VVIS_AUTO_FAST_MAX_PORTALS,
        };

        assert_eq!(
            parse_prt_portal_counts("PRT1\n7492\n31725\n4 0 1\n"),
            Some((7492, 31725))
        );
        assert_eq!(
            parse_prt_portal_counts("PRT2\r\n120\r\n340\r\n"),
            Some((120, 340))
        );
        assert_eq!(parse_prt_portal_counts(""), None);
        assert_eq!(parse_prt_portal_counts("NOT_PRT\n10\n20\n"), None);
        assert_eq!(parse_prt_portal_counts("PRT1\nabc\n20\n"), None);

        assert!(!should_use_fast_vvis(false, Some((300, 900))));
        assert!(!should_use_fast_vvis(
            false,
            Some((VVIS_AUTO_FAST_MAX_CLUSTERS, VVIS_AUTO_FAST_MAX_PORTALS))
        ));
        assert!(should_use_fast_vvis(
            false,
            Some((VVIS_AUTO_FAST_MAX_CLUSTERS + 1, 500))
        ));
        assert!(should_use_fast_vvis(
            false,
            Some((200, VVIS_AUTO_FAST_MAX_PORTALS + 1))
        ));
        assert!(should_use_fast_vvis(true, Some((10, 20))));
        assert!(!should_use_fast_vvis(false, None));
        assert!(should_use_fast_vvis(true, None));
    }

    #[test]
    fn test_core_occluders_volume_filter_and_cap() {
        use crate::collision::{
            select_top_core_occluders, MAX_CORE_OCCLUDERS, MIN_CORE_OCCLUDER_CELLS,
        };

        let mut boxes = Vec::new();
        // Tiny 1x1x1 macro cells (vol = 1 < MIN_CORE_OCCLUDER_CELLS) must be filtered out
        for i in 0..20 {
            boxes.push([i, 0, 0, i + 1, 1, 1]);
        }
        // 70 larger boxes with increasing volumes (vol >= 4)
        for i in 1..=70 {
            boxes.push([0, i, 0, 2, i + 2, i + 1]); // vol = 2 * 2 * (i + 1) >= 8
        }

        let selected =
            select_top_core_occluders(boxes, MAX_CORE_OCCLUDERS, MIN_CORE_OCCLUDER_CELLS);
        assert_eq!(selected.len(), MAX_CORE_OCCLUDERS);
        let box_vol = |b: [usize; 6]| (b[3] - b[0]) * (b[4] - b[1]) * (b[5] - b[2]);
        for b in &selected {
            assert!(box_vol(*b) >= MIN_CORE_OCCLUDER_CELLS);
        }
        assert_eq!(box_vol(selected[0]), 2 * 2 * 71);
        for pair in selected.windows(2) {
            assert!(box_vol(pair[0]) >= box_vol(pair[1]));
        }
    }

    #[test]
    fn test_vmf_prioritizes_largest_core_occluders_and_water_pools() {
        use crate::vmf_builder::build_vmf_checked;

        let tmp = std::env::temp_dir().join(format!("mcgm_test_vmf_prio_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let vmf_path = tmp.join("prio.vmf");

        let mut core_occluders = Vec::new();
        for i in 0..60 {
            let x = -4000.0 + (i as f64) * 20.0;
            core_occluders.push([x, 0.0, 0.0, x + 160.0, 160.0, 160.0]);
        }
        for i in 0..20 {
            let x = 1000.0 + (i as f64) * 20.0;
            core_occluders.push([x, 0.0, 0.0, x + 800.0, 800.0, 800.0]);
        }

        let mut water_pools = Vec::new();
        for i in 0..1024 {
            let x = -4000.0 + (i as f64) * 4.0;
            water_pools.push([x, -500.0, 10.0, x + 40.0, -460.0, 45.0]);
        }
        for i in 0..6 {
            let x = 2000.0 + (i as f64) * 500.0;
            water_pools.push([x, 500.0, 100.0, x + 400.0, 900.0, 300.0]);
        }

        let problems = build_vmf_checked(&VmfBuildParams {
            output_vmf: &vmf_path,
            category: "custom_mc",
            bounds_min: [-5000.0, -5000.0, 0.0],
            bounds_max: [6000.0, 5000.0, 1000.0],
            chunks: &[],
            all_solid_boxes: &[],
            ladder_boxes: &[],
            water_pools: &water_pools,
            hazard_boxes: &[],
            core_occluders: &core_occluders,
            doors: &[],
            buttons: &[],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 64.0]],
            lighting_style: "minecraft",
            hq_light: true,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();

        assert!(
            problems.iter().any(|p| p.contains("Water pools truncated: emitted 1024 of 1030")),
            "Expected water truncation warning, got {:?}",
            problems
        );

        let vmf_text = std::fs::read_to_string(&vmf_path).unwrap();
        let occ_count = vmf_text.matches("\"color\" \"180 180 0\"").count();
        assert_eq!(occ_count, 48, "VMF must cap core occluders at 48");
        assert!(
            vmf_text.contains("(1380.00 800.00 800.00)"),
            "Largest core occluder must be prioritized over earlier smaller ones"
        );

        let water_count = vmf_text.matches("custom_mc/mc_water").count();
        assert_eq!(water_count, 1024, "VMF must emit 1024 water brushes");
        assert!(
            vmf_text.contains("(2000.00 900.00 300.00)"),
            "Largest water lake must be prioritized over earlier 1-block puddles"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_vrad_args_skips_cpu_propambient_and_keeps_gpu_hq() {
        use crate::compiler::build_vrad_args;

        // 1. VRAD++ HQ GPU: no CPU-bound -propambient, no -fast/-fastambient/-cpu
        let hq_args = build_vrad_args(true, "-ldr", true, false, false, "minecraft");
        assert!(!hq_args.iter().any(|a| a == "-propambient"));
        assert!(!hq_args.iter().any(|a| a == "-fast"));
        assert!(!hq_args.iter().any(|a| a == "-fastambient"));
        assert!(!hq_args.iter().any(|a| a == "-cpu"));
        for expected in [
            "-ldr",
            "-StaticPropLighting",
            "-StaticPropPolys",
            "-textureshadows",
            "-forcetextureshadows",
            "-ao",
            "-aoradius",
            "30",
            "-aoscale",
            "0.25",
            "-aopropsamples",
            "24",
            "-ambient",
            "0.04",
            "0.05",
            "0.06",
        ] {
            assert!(
                hq_args.iter().any(|a| a == expected),
                "HQ VRAD++ args missing {}: {:?}",
                expected,
                hq_args
            );
        }

        // 2. VRAD++ fast: includes -fast, -fastambient, -aopropsamples 12; gmod omits -ambient
        let fast_args = build_vrad_args(true, "-ldr", false, true, false, "gmod");
        assert!(!fast_args.iter().any(|a| a == "-propambient"));
        assert!(!fast_args.iter().any(|a| a == "-ambient"));
        for expected in ["-fast", "-fastambient", "-aopropsamples", "12"] {
            assert!(
                fast_args.iter().any(|a| a == expected),
                "Fast VRAD++ args missing {}: {:?}",
                expected,
                fast_args
            );
        }

        // 3. VRAD++ CPU fallback: includes -cpu, -fastambient, -aopropsamples 24
        let cpu_args = build_vrad_args(true, "-hdr", true, false, true, "night");
        assert!(!cpu_args.iter().any(|a| a == "-propambient"));
        for expected in ["-hdr", "-cpu", "-fastambient", "-aopropsamples", "24"] {
            assert!(
                cpu_args.iter().any(|a| a == expected),
                "CPU fallback VRAD++ args missing {}: {:?}",
                expected,
                cpu_args
            );
        }

        // 4. Stock VRAD
        let stock_hq = build_vrad_args(false, "-both", true, false, false, "minecraft");
        assert!(!stock_hq.iter().any(|a| a == "-ao" || a == "-propambient"));
        for expected in ["-both", "-StaticPropLighting", "-StaticPropPolys", "-TextureShadows"] {
            assert!(stock_hq.iter().any(|a| a == expected));
        }

        let stock_fast = build_vrad_args(false, "-ldr", false, true, false, "minecraft");
        for expected in ["-ldr", "-StaticPropLighting", "-fast"] {
            assert!(stock_fast.iter().any(|a| a == expected));
        }
    }

    #[test]
    fn test_debug_log_path_resolution_and_command_formatting() {
        use crate::cli::CliArgs;
        use crate::compiler::format_command_debug;
        use crate::resolve_debug_log_path;
        use clap::Parser;
        use std::path::{Path, PathBuf};

        assert_eq!(
            resolve_debug_log_path(Some(Path::new("explicit.log")), Some("env.log"), Path::new("work")),
            PathBuf::from("explicit.log")
        );
        assert_eq!(
            resolve_debug_log_path(None, Some("env.log"), Path::new("work")),
            PathBuf::from("env.log")
        );
        assert_eq!(
            resolve_debug_log_path(None, Some("   "), Path::new("work")),
            PathBuf::from("work").join("mcgm_debug.log")
        );
        assert_eq!(
            resolve_debug_log_path(None, None, Path::new("work")),
            PathBuf::from("work").join("mcgm_debug.log")
        );

        let cmd_str = format_command_debug(
            Path::new(r"C:\My Tools\vrad++.exe"),
            &[
                "-ldr".to_string(),
                "-game".to_string(),
                r"C:\Program Files\GMod\garrysmod".to_string(),
            ],
        );
        assert_eq!(
            cmd_str,
            "\"C:\\My Tools\\vrad++.exe\" -ldr -game \"C:\\Program Files\\GMod\\garrysmod\""
        );

        let parsed = CliArgs::try_parse_from(["mcgm", "--debug-log", "logs/test.log"]).unwrap();
        assert_eq!(parsed.debug_log, Some(PathBuf::from("logs/test.log")));
    }

    #[test]
    fn test_water_brush_eligibility_excludes_flowing_and_solid_waterlogged() {
        use crate::collision::{build_world_collision_with_chunk_size, is_water_brush_eligible};

        let still_water = make_state("minecraft:water", &[("level", "0")]);
        let default_water = make_state("minecraft:water", &[]);
        let flowing_lvl3 = make_state("minecraft:water", &[("level", "3")]);
        let flowing_water = make_state("minecraft:flowing_water", &[("level", "0")]);
        let waterlogged_stairs = make_state("minecraft:oak_stairs", &[("waterlogged", "true")]);
        let waterlogged_wall = make_state("minecraft:cobblestone_wall", &[("waterlogged", "true")]);
        let waterlogged_pickle = make_state("minecraft:sea_pickle", &[("waterlogged", "true")]);

        assert!(is_water_brush_eligible(&still_water));
        assert!(is_water_brush_eligible(&default_water));
        assert!(!is_water_brush_eligible(&flowing_lvl3));
        assert!(!is_water_brush_eligible(&flowing_water));
        assert!(!is_water_brush_eligible(&waterlogged_stairs));
        assert!(!is_water_brush_eligible(&waterlogged_wall));
        assert!(is_water_brush_eligible(&waterlogged_pickle));

        let air = make_state("minecraft:air", &[]);
        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 5,
            max_y: 64,
            max_z: 0,
        };
        let mut blocks = vec![0u16; coords.size_x() * coords.size_y() * coords.size_z()];
        blocks[0] = 1; // still_water at (0, 64, 0)
        blocks[2] = 2; // flowing_lvl3 at (2, 64, 0)
        blocks[4] = 3; // waterlogged_stairs at (4, 64, 0)
        let grid = VoxelGrid {
            coords,
            palette: vec![air, still_water, flowing_lvl3, waterlogged_stairs],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 6],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision_with_chunk_size(&grid, [0.0, 64.0, 0.0], 39.37, 64);
        assert_eq!(col.water_pools.len(), 1, "Only still level=0 water emits a BSP water brush");
        assert_eq!(
            col.physics_zones.iter().filter(|z| z.kind == "water").count(),
            3,
            "All 3 water blocks (still, flowing, waterlogged) must keep GLua water physics zones"
        );
    }

    #[test]
    fn test_anvil_decompress_rejects_oversized_gzip_and_zlib() {
        use flate2::write::{GzEncoder, ZlibEncoder};
        use flate2::Compression;
        use std::io::Write;

        let zeros = vec![0u8; 64 * 1024 * 1024 + 1024];

        let mut gz_enc = GzEncoder::new(Vec::new(), Compression::fast());
        gz_enc.write_all(&zeros).unwrap();
        let gz_bytes = gz_enc.finish().unwrap();
        assert!(
            crate::anvil::decompress_chunk_payload(1, &gz_bytes).is_none(),
            "Gzip payload exceeding 64 MiB must be rejected"
        );

        let mut zl_enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        zl_enc.write_all(&zeros).unwrap();
        let zl_bytes = zl_enc.finish().unwrap();
        assert!(
            crate::anvil::decompress_chunk_payload(2, &zl_bytes).is_none(),
            "Zlib payload exceeding 64 MiB must be rejected"
        );
    }

    #[test]
    fn test_assets_rejects_oversized_zip_entry() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let tmp = make_temp_test_dir("oversized_zip");
        let jar_path = tmp.join("oversized_version.jar");
        {
            let f = std::fs::File::create(&jar_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("version.json", opts).unwrap();
            let mut body = b"{\"world_version\": 3953, \"pad\": \"".to_vec();
            body.extend(std::iter::repeat_n(b'a', 1024 * 1024 + 512));
            body.extend_from_slice(b"\"}");
            zw.write_all(&body).unwrap();
            zw.finish().unwrap();
        }
        assert!(
            crate::assets::read_jar_world_version(&jar_path).is_none(),
            "Oversized version.json (>1 MiB) must be rejected"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_font_renderer_handles_tiny_and_zero_sheets_without_panic() {
        use crate::assets::AssetIndex;
        use crate::atlas::{measure_glyph_width, FontRenderer};
        use image::RgbaImage;

        let tiny = RgbaImage::new(1, 1);
        assert_eq!(measure_glyph_width(&tiny, 0, 0, 8, 8), 4);
        let empty = RgbaImage::new(0, 0);
        assert_eq!(measure_glyph_width(&empty, 0, 0, 8, 8), 4);

        let mut assets = AssetIndex::empty();
        let mut png_bytes = Vec::new();
        tiny.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        assets.insert_entry("assets/minecraft/textures/font/tiny.png", png_bytes);
        assets.insert_entry(
            "assets/minecraft/font/default.json",
            br#"{"providers":[{"type":"bitmap","file":"minecraft:font/tiny.png","chars":["ABCD","EFGH"]}]}"#.to_vec(),
        );
        let _fr = FontRenderer::from_assets(&assets);
    }

    #[test]
    fn test_qc_cache_hash_invalidated_when_body_smd_changes() {
        use crate::compiler::{qc_cache_signature_matches, write_qc_cache_signature};

        let tmp = make_temp_test_dir("qc_body_smd_hash");
        let qc = tmp.join("chunk_0_0.qc");
        let body_smd = tmp.join("chunk_0_0.smd");
        let phys_smd = tmp.join("chunk_0_0_phys.smd");
        let addon_mdl = tmp.join("chunk_0_0.mdl");

        std::fs::write(&body_smd, "version 1\nnodes\nend\ntriangles\nmat1\nend\n").unwrap();
        std::fs::write(&phys_smd, "version 1\nend\n").unwrap();
        std::fs::write(
            &qc,
            "$modelname \"mcgm/chunk_0_0.mdl\"\n$body \"studio\" \"chunk_0_0.smd\"\n$surfaceprop \"dirt\"\n$collisionmodel \"chunk_0_0_phys.smd\" {\n}\n",
        )
        .unwrap();
        std::fs::write(&addon_mdl, b"IDST").unwrap();

        write_qc_cache_signature(&qc, &addon_mdl);
        assert!(qc_cache_signature_matches(&qc, &addon_mdl));

        // Modify only the visual $body SMD without touching .qc or _phys.smd
        std::fs::write(&body_smd, "version 1\nnodes\nend\ntriangles\nmat2_changed\nend\n").unwrap();
        assert!(
            !qc_cache_signature_matches(&qc, &addon_mdl),
            "changing $body SMD must invalidate QC cache signature"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_water_and_animated_materials_respect_no_import_to_gmod() {
        use crate::atlas::{AnimatedMaterialEntry, RenderBucket};
        use crate::compiler::{compile_animated_materials, compile_water_materials, ToolPaths};
        use image::RgbaImage;

        let tmp = make_temp_test_dir("no_import_mats");
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        let game_dir = tmp.join("garrysmod");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();

        let still_png = work_dir.join("mc_water.png");
        let flow_png = work_dir.join("mc_water_flow.png");
        crate::atlas::save_png_fast(&RgbaImage::new(16, 16), &still_png).unwrap();
        crate::atlas::save_png_fast(&RgbaImage::new(16, 16), &flow_png).unwrap();

        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            "@echo off\r\nfor %%F in (%2) do echo DUMMY > \"%~4\\%%~nF.vtf\"\r\n",
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: game_dir.clone(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };

        compile_water_materials(&tools, "custom_mc", &still_png, &flow_png, &addon_dir, false).unwrap();
        assert!(addon_dir.join("materials/custom_mc/mc_water.vmt").is_file());
        assert!(
            !game_dir.join("materials/custom_mc/mc_water.vmt").exists(),
            "compile_water_materials must NOT write to game_dir when import_to_gmod == false"
        );

        let anim_entries = vec![AnimatedMaterialEntry {
            mat_name: "anim_lava".to_string(),
            frames: vec![RgbaImage::new(16, 16)],
            fps: 4.0,
            bucket: RenderBucket::TerrainOpaque,
            unlit_emissive: true,
        }];
        compile_animated_materials(&tools, "custom_mc", &anim_entries, &addon_dir, false).unwrap();
        assert!(addon_dir.join("materials/models/custom_mc/anim_lava.vmt").is_file());
        assert!(
            !game_dir.join("materials/models/custom_mc/anim_lava.vmt").exists(),
            "compile_animated_materials must NOT write to game_dir when import_to_gmod == false"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_is_snapshot_token_handles_non_ascii_safely() {
        assert!(!crate::instances::is_snapshot_token("24w1\u{2603}a"));
        assert!(!crate::instances::is_snapshot_token("24w\u{2603}a"));
        assert!(crate::instances::is_snapshot_token("24w14a"));
    }

    #[test]
    fn test_baked_face_preserves_per_biome_material_and_uvs() {
        use crate::assets::{FaceDir, ModelElement, ModelFace, ResolvedBlockModel};
        use crate::atlas::{
            build_texture_atlas, RenderBucket, SignAtlas, TexRequestKey,
        };
        use crate::mesher::{stream_world_chunks_with_animated, ExtractedFeatures};
        use image::{ImageBuffer, Rgba, RgbaImage};
        use std::collections::HashSet;

        let air = make_state("minecraft:air", &[]);
        let leaf = make_state("minecraft:oak_leaves", &[]);

        let mut faces = HashMap::new();
        faces.insert(
            FaceDir::Up,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/oak_leaves".to_string(),
                cullface: None,
                rotation: 0,
                tint_index: Some(0),
            },
        );
        let model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.0, 0.0, 0.0],
            to: [16.0, 16.0, 16.0],
            rotation: None,
            faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);

        let mut assets = crate::assets::AssetIndex::empty();
        let gray_img: RgbaImage = ImageBuffer::from_pixel(16, 16, Rgba([200, 200, 200, 255]));
        let mut png_bytes = Vec::new();
        gray_img
            .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .unwrap();
        assets.insert_entry("assets/minecraft/textures/block/oak_leaves.png", png_bytes);

        let plains_key = TexRequestKey {
            tex_ref: "minecraft:block/oak_leaves".to_string(),
            tint_rgb: Some([119, 171, 47]),
        };
        let desert_key = TexRequestKey {
            tex_ref: "minecraft:block/oak_leaves".to_string(),
            tint_rgb: Some([191, 183, 85]),
        };

        let mut req_keys = HashSet::new();
        req_keys.insert(desert_key.clone());
        let atlas = build_texture_atlas(&req_keys, &assets, 2);
        let sign_atlas = SignAtlas {
            image: RgbaImage::new(64, 64),
            rects: HashMap::new(),
        };

        // Biome 0 (plains) uses standalone animated material; Biome 1 (desert) falls back to atlas.
        let mut animated_mats = HashMap::new();
        animated_mats.insert(
            plains_key,
            ("anim_plains_leaf".to_string(), RenderBucket::TerrainCutout),
        );

        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 1,
            max_y: 64,
            max_z: 0,
        };
        let grid = VoxelGrid {
            coords,
            palette: vec![air, leaf],
            air_id: 0,
            blocks: vec![1, 1],
            biome_palette: vec![
                "minecraft:plains".to_string(),
                "minecraft:desert".to_string(),
            ],
            biomes: vec![0, 1],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let features = ExtractedFeatures {
            lights: Vec::new(),
            doors: Vec::new(),
            door_block_names: HashSet::new(),
            door_voxels: HashSet::new(),
            flat_water_voxels: HashSet::new(),
            buttons: Vec::new(),
            spawn_points_mc: Vec::new(),
            skip_mesh_bits: Vec::new(),
        };
        let chunks = stream_world_chunks_with_animated(
            &grid,
            &[ResolvedBlockModel::Empty, model],
            &atlas,
            &sign_atlas,
            &features,
            [0.0, 64.0, 0.0],
            39.37,
            64,
            &animated_mats,
            Ok,
        )
        .unwrap();

        let desert_rect = atlas.get_or_default(&desert_key);
        let tris = &chunks[0].solid_tris;
        // Desert block is at wx=1 -> Source X > 20; Plains block is at wx=0 -> Source X < 20
        let desert_tri = tris
            .iter()
            .find(|t| t.pos[0][0] > 20.0)
            .expect("desert block tri");
        assert_ne!(
            desert_tri.material.as_ref(),
            "anim_plains_leaf",
            "Desert biome face must not inherit biome 0's animated material"
        );
        assert_eq!(desert_tri.material.as_ref(), desert_rect.bucket.material_name());
    }

    #[test]
    fn test_shelf_pack_best_effort_does_not_overwrite_origin_on_overflow() {
        let placements = crate::atlas::shelf_pack_best_effort(&[(16, 16), (9000, 9000)], 8192, 8192);
        assert_ne!(
            placements[0], placements[1],
            "Overflowed tile must not collide with (0, 0)"
        );
    }

    #[test]
    fn test_world_coords_size_handles_extreme_i32_bounds() {
        let coords = WorldCoords {
            min_x: i32::MIN + 1,
            min_y: 0,
            min_z: 0,
            max_x: 10,
            max_y: 0,
            max_z: 0,
        };
        assert!(coords.size_x() > 0);
    }

    #[test]
    fn test_sign_renderer_hd_glyph_tall_line_not_clipped() {
        use crate::atlas::{BitmapGlyph, FontRenderer};
        use image::{ImageBuffer, Rgba, RgbaImage};
        use std::sync::Arc;

        // 16x16 HD glyph with opaque pixels in rows 12..15 (well below 9px)
        let mut sheet: RgbaImage = ImageBuffer::new(16, 16);
        for y in 12..16 {
            for x in 2..10 {
                sheet.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let mut glyphs = HashMap::new();
        glyphs.insert(
            'A',
            BitmapGlyph {
                sheet: Arc::new(sheet),
                gx: 0,
                gy: 0,
                gw: 16,
                gh: 16,
                content_width: 12,
            },
        );
        let fr = FontRenderer { glyphs };
        // Long line (> 244px at 1x) triggers the downscale branch
        let long_line = "A".repeat(25);
        let empty_bg: RgbaImage = ImageBuffer::from_pixel(64, 32, Rgba([100, 80, 60, 255]));
        let rendered = fr.render_sign_panel_with_base(
            &[long_line, String::new(), String::new(), String::new()],
            "white",
            [100, 80, 60],
            Some(&empty_bg),
            false,
        );
        // Should have white ink pixels from rows 12..15 of the HD glyph
        let has_white = rendered.pixels().any(|p| p[0] > 230 && p[1] > 230 && p[2] > 230);
        assert!(has_white, "HD glyph rows >= 9 must not be clipped on long sign lines");
    }

    #[test]
    fn test_sea_lantern_is_full_opaque_not_partial() {
        use crate::collision::build_world_collision_with_chunk_size;

        let air = make_state("minecraft:air", &[]);
        let sea_lantern = make_state("minecraft:sea_lantern", &[]);
        let lantern = make_state("minecraft:lantern", &[]);

        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 1,
            max_y: 64,
            max_z: 2,
        };
        let mut blocks = vec![0u16; coords.size_x() * coords.size_y() * coords.size_z()];
        // Two adjacent sea_lanterns at (0, 64, 0) and (1, 64, 0) -> z=0
        blocks[0] = 1;
        blocks[1] = 1;
        // Two adjacent lanterns at (0, 64, 2) and (1, 64, 2) -> z=2 (offset 4, 5)
        blocks[4] = 2;
        blocks[5] = 2;

        let grid = VoxelGrid {
            coords,
            palette: vec![air, sea_lantern, lantern],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 6],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision_with_chunk_size(&grid, [0.0, 64.0, 0.0], 39.37, 64);
        // 2 sea_lanterns should merge into 1 solid box + 2 unmerged lanterns = 3 boxes total
        assert_eq!(
            col.all_solid_boxes.len(),
            3,
            "sea_lantern must merge as FullOpaque (1 box), while lantern stays Partial (2 boxes)"
        );
    }

    #[test]
    fn test_vmf_validator_flags_brushes_over_8192() {
        let mut v = crate::vmf_builder::VmfValidator::new();
        for _ in 0..8193 {
            v.record_solid(6);
        }
        let problems = v.into_problems();
        assert!(
            problems.iter().any(|p| p.contains("8192")),
            "VmfValidator must flag solid_count > 8192, got: {:?}",
            problems
        );
    }

    #[test]
    fn test_cli_sanitizes_traversal_in_name_map_category() {
        use crate::cli::CliArgs;
        use clap::Parser;

        let args = CliArgs::parse_from([
            "mcgm",
            "--name",
            "../../evil",
            "--map",
            "../pwn",
            "--category",
            "../../Windows",
        ]);
        let n = args.effective_name();
        let m = args.effective_map();
        assert!(!n.contains("..") && !n.contains('/') && !n.contains('\\'));
        assert!(!m.contains("..") && !m.contains('/') && !m.contains('\\'));
    }

    #[test]
    fn test_mcgm_physics_finishmove_syncs_sp_step_state() {
        let lua = crate::glua::generate_mcgm_physics_autorun();
        let fm_start = lua
            .find("hook.Add(\"FinishMove\"")
            .expect("FinishMove hook present");
        let cv = lua[fm_start..]
            .find("MCGM_PhysicsStepCamera")
            .expect("CalcView");
        let fm_body = &lua[fm_start..fm_start + cv];
        assert!(
            fm_body.contains("ply._mcgmSpLastZ = pos.z"),
            "FinishMove must sync ply._mcgmSpLastZ = pos.z to avoid MP CalcView double-subtraction"
        );
    }

    #[test]
    fn test_sanitize_map_stem_blocks_reserved_dos_names() {
        for dos in ["CON", "nul", "COM1", "lpt9", "aux"] {
            let stem = crate::glua::sanitize_map_stem(dos);
            assert_ne!(
                stem.to_ascii_uppercase(),
                dos.to_ascii_uppercase(),
                "sanitize_map_stem must not return bare reserved DOS name '{}'",
                dos
            );
        }
    }

    #[test]
    fn test_resolve_world_in_instance_prefers_saves_dir_over_cwd() {
        use crate::instances::{resolve_world_in_instance, LauncherKind, MinecraftInstance};

        let tmp = make_temp_test_dir("resolve_world_cwd");
        let saves_dir = tmp.join("saves");
        // "src" exists as a directory in the repo CWD, so a relative "src" query
        // must resolve to saves_dir/src rather than CWD/src.
        let expected_world = make_dummy_world(&saves_dir, "src");
        let inst = MinecraftInstance {
            launcher: LauncherKind::Vanilla,
            name: "test".to_string(),
            instance_dir: tmp.clone(),
            saves_dir: saves_dir.clone(),
            mc_version: None,
            loader: None,
            worlds: Vec::new(),
        };
        let resolved = resolve_world_in_instance(&inst, Some(std::path::Path::new("src")))
            .expect("should resolve world inside saves_dir");
        assert_eq!(resolved, expected_world);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_gui_tr_text_full_key_parity() {
        let all_keys = [
            "win_title", "ui_title", "step_world", "step_area", "step_map",
            "rad_small", "rad_medium", "rad_large", "lbl_preset", "preset_preview",
            "preset_balanced", "preset_hq", "preset_custom", "grp_src", "grp_meta",
            "grp_opts", "grp_adv", "lbl_log_hdr", "lbl_world", "lbl_saves",
            "lbl_min", "lbl_max", "lbl_dim", "lbl_scale", "lbl_name", "lbl_cat",
            "lbl_map", "lbl_outdir", "lbl_light", "lbl_instance", "instances_all",
            "world_custom", "chk_import", "chk_vmf", "chk_hq", "chk_fast",
            "chk_phys", "chk_brushes", "btn_browse", "btn_adv_show", "btn_adv_hide",
            "btn_log_show", "btn_log_hide", "btn_start", "btn_running", "btn_stop",
            "btn_open_out", "btn_launch_gmod", "btn_lang", "status_ready",
            "status_done", "status_failed", "status_cancelled", "sel_empty",
        ];
        for key in all_keys {
            let ru = crate::gui::tr_text(crate::gui::UiLang::Ru, key);
            let en = crate::gui::tr_text(crate::gui::UiLang::En, key);
            assert!(!ru.is_empty(), "Missing RU key: {}", key);
            assert!(!en.is_empty(), "Missing EN key: {}", key);
            if !matches!(key, "win_title" | "rad_small" | "rad_medium" | "rad_large") {
                assert_ne!(ru, en, "RU and EN should be localized for key: {}", key);
            }
        }
    }

    #[test]
    fn test_mc_26_default_empty_props_stairs_logs_and_walls() {
        fn push_tag_header(buf: &mut Vec<u8>, tag_type: u8, name: &str) {
            buf.push(tag_type);
            let nb = name.as_bytes();
            buf.extend_from_slice(&(nb.len() as u16).to_be_bytes());
            buf.extend_from_slice(nb);
        }
        fn push_nbt_str(buf: &mut Vec<u8>, val: &str) {
            let vb = val.as_bytes();
            buf.extend_from_slice(&(vb.len() as u16).to_be_bytes());
            buf.extend_from_slice(vb);
        }
        fn push_int(buf: &mut Vec<u8>, name: &str, v: i32) {
            push_tag_header(buf, 3, name);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        fn push_byte(buf: &mut Vec<u8>, name: &str, v: i8) {
            push_tag_header(buf, 1, name);
            buf.push(v as u8);
        }
        fn push_str_tag(buf: &mut Vec<u8>, name: &str, val: &str) {
            push_tag_header(buf, 8, name);
            push_nbt_str(buf, val);
        }

        // Construct MC 26.3 (DataVersion 5023) chunk where default-state blocks omit Properties
        let mut nbt = Vec::new();
        push_tag_header(&mut nbt, 10, "");
        push_int(&mut nbt, "DataVersion", 5023);
        push_int(&mut nbt, "xPos", 0);
        push_int(&mut nbt, "zPos", 0);

        push_tag_header(&mut nbt, 9, "sections");
        nbt.push(10);
        nbt.extend_from_slice(&1i32.to_be_bytes());
        push_byte(&mut nbt, "Y", 4);

        push_tag_header(&mut nbt, 10, "block_states");
        push_tag_header(&mut nbt, 9, "palette");
        nbt.push(10);
        nbt.extend_from_slice(&4i32.to_be_bytes());
        // palette[0] = minecraft:air
        push_str_tag(&mut nbt, "Name", "minecraft:air");
        nbt.push(0);
        // palette[1] = {"": "minecraft:spruce_stairs"} (no Properties)
        push_str_tag(&mut nbt, "", "minecraft:spruce_stairs");
        nbt.push(0);
        // palette[2] = {"Name": "minecraft:oak_log"} (no Properties)
        push_str_tag(&mut nbt, "Name", "minecraft:oak_log");
        nbt.push(0);
        // palette[3] = {"Name": "minecraft:stone_brick_wall"} (no Properties)
        push_str_tag(&mut nbt, "Name", "minecraft:stone_brick_wall");
        nbt.push(0);

        let mut longs = vec![0i64; 256];
        // (0, 64, 0) -> idx 0 -> palette[1] (spruce_stairs)
        longs[0] |= 1i64;
        // (1, 64, 0) -> idx 1 -> palette[2] (oak_log)
        longs[0] |= 2i64 << 4;
        // (2, 64, 0) -> idx 2 -> palette[3] (stone_brick_wall)
        longs[0] |= 3i64 << 8;

        push_tag_header(&mut nbt, 12, "data");
        nbt.extend_from_slice(&(longs.len() as i32).to_be_bytes());
        for l in longs {
            nbt.extend_from_slice(&l.to_be_bytes());
        }
        nbt.push(0); // end block_states

        push_tag_header(&mut nbt, 10, "biomes");
        push_tag_header(&mut nbt, 9, "palette");
        nbt.push(8);
        nbt.extend_from_slice(&1i32.to_be_bytes());
        push_nbt_str(&mut nbt, "minecraft:plains");
        nbt.push(0);

        nbt.push(0); // end section 0
        nbt.push(0); // end root

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&nbt).unwrap();
        let compressed = enc.finish().unwrap();

        let payload_len = (compressed.len() + 1) as u32;
        let total_chunk_bytes = 4 + 1 + compressed.len();
        let sector_count = total_chunk_bytes.div_ceil(4096) as u8;
        let mut mca = vec![0u8; 8192 + (sector_count as usize) * 4096];
        mca[0] = 0;
        mca[1] = 0;
        mca[2] = 2;
        mca[3] = sector_count;
        mca[8192..8196].copy_from_slice(&payload_len.to_be_bytes());
        mca[8196] = 2;
        mca[8197..8197 + compressed.len()].copy_from_slice(&compressed);

        let world_dir = std::env::temp_dir().join(format!("mcgm_test_mc26_props_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world_dir);
        let region_dir = world_dir.join("region");
        std::fs::create_dir_all(&region_dir).unwrap();
        std::fs::write(region_dir.join("r.0.0.mca"), &mca).unwrap();

        let coords = WorldCoords {
            min_x: 0,
            min_y: 64,
            min_z: 0,
            max_x: 3,
            max_y: 65,
            max_z: 3,
        };
        let grid = load_world_region(&world_dir, "overworld", coords, 39.37).expect("MC 26.3 load");
        let _ = std::fs::remove_dir_all(&world_dir);

        let stairs = grid.get_block(0, 64, 0);
        assert_eq!(stairs.name, "minecraft:spruce_stairs");
        assert_eq!(stairs.prop("facing"), Some("north"));
        assert_eq!(stairs.prop("half"), Some("bottom"));
        assert_eq!(stairs.prop("shape"), Some("straight"));
        assert_eq!(stairs.prop("waterlogged"), Some("false"));

        let log = grid.get_block(1, 64, 0);
        assert_eq!(log.name, "minecraft:oak_log");
        assert_eq!(log.prop("axis"), Some("y"));

        let wall = grid.get_block(2, 64, 0);
        assert_eq!(wall.name, "minecraft:stone_brick_wall");
        assert_eq!(wall.prop("up"), Some("true"));
        assert_eq!(wall.prop("north"), Some("none"));
        assert_eq!(wall.prop("waterlogged"), Some("false"));
    }

    #[test]
    fn test_match_best_variant_prefers_canonical_defaults_over_alphabetical() {
        let mut assets = AssetIndex::empty();
        assets.insert_entry(
            "assets/minecraft/blockstates/oak_stairs.json",
            br#"{
                "variants": {
                    "facing=east,half=bottom,shape=inner_left": {"model": "minecraft:block/oak_stairs", "y": 90},
                    "facing=north,half=bottom,shape=straight": {"model": "minecraft:block/oak_stairs"},
                    "facing=west,half=bottom,shape=straight": {"model": "minecraft:block/oak_stairs", "y": 270}
                }
            }"#.to_vec(),
        );
        assets.insert_entry(
            "assets/minecraft/models/block/oak_stairs.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"minecraft:block/oak_planks"}}}]}"##.to_vec(),
        );

        let empty_stairs = make_state("minecraft:oak_stairs", &[]);
        match assets.resolve_block_model(&empty_stairs) {
            ResolvedBlockModel::Elements(els) => {
                assert_eq!(
                    els[0].rot_y, 0,
                    "Empty props must prefer canonical facing=north,half=bottom,shape=straight (rot_y=0), not alphabetical facing=east,inner_left"
                );
            }
            other => panic!("Expected Elements, got {:?}", other),
        }

        let partial_stairs = make_state("minecraft:oak_stairs", &[("half", "bottom")]);
        match assets.resolve_block_model(&partial_stairs) {
            ResolvedBlockModel::Elements(els) => {
                assert_eq!(
                    els[0].rot_y, 0,
                    "Partial props (half=bottom) must prefer facing=north,shape=straight (rot_y=0)"
                );
            }
            other => panic!("Expected Elements, got {:?}", other),
        }
    }

    #[test]
    fn test_fence_gate_and_button_do_not_use_full_atlas_brush() {
        use crate::mesher::extract_features_and_requests;

        let air = make_state("minecraft:air", &[]);
        let gate = make_state("minecraft:oak_fence_gate", &[("facing", "north"), ("open", "false")]);
        let btn = make_state("minecraft:stone_button", &[("facing", "north"), ("face", "wall")]);
        let lever = make_state("minecraft:lever", &[("facing", "north"), ("face", "wall")]);
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 2, max_y: 64, max_z: 0 },
            palette: vec![air, gate, btn, lever],
            air_id: 0,
            blocks: vec![1, 2, 3],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 3],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty; grid.palette.len()];
        let (feat, _reqs) = extract_features_and_requests(&grid, &models, [0.0, 64.0, 0.0], 39.37);
        assert_eq!(feat.doors.len(), 1);
        assert_ne!(feat.doors[0].mat_name, "atlas_brush", "Fence gate must use its own tile material, not atlas_brush");
        assert_eq!(feat.buttons.len(), 2);
        for b in &feat.buttons {
            assert_ne!(b.material, "atlas_brush", "Button/lever '{}' must not use atlas_brush", b.kind);
        }
    }

    #[test]
    fn test_empty_sign_uses_wood_base_not_solid_fallback() {
        use crate::atlas::build_sign_atlas_with_assets;
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        let mut assets = AssetIndex::empty();
        // Give spruce planks a distinct non-solid patterned 16x16 texture
        let mut planks_img = ImageBuffer::from_pixel(16, 16, Rgba([114, 84, 48, 255]));
        for y in 0..16u32 {
            for x in 0..16u32 {
                if (x + y) % 3 == 0 {
                    planks_img.put_pixel(x, y, Rgba([92, 64, 34, 255]));
                }
            }
        }
        assets.insert_entry("assets/minecraft/textures/block/spruce_planks.png", encode_png_bytes(&planks_img));

        let air = make_state("minecraft:air", &[]);
        let spruce_sign = make_state("minecraft:spruce_wall_sign", &[("facing", "north")]);
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 0, max_y: 64, max_z: 0 },
            palette: vec![air, spruce_sign],
            air_id: 0,
            blocks: vec![1],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0],
            signs: HashMap::new(), // Decorative empty sign without BlockEntity
            player_spawn_mc: None,
        };
        let font = FontRenderer::from_assets(&assets);
        let sign_atlas = build_sign_atlas_with_assets(&grid, &assets, &font);
        assert!(
            !sign_atlas.rects.is_empty(),
            "Empty/decorative signs in palette must still produce wood base rects in SignAtlas"
        );

        let models = vec![ResolvedBlockModel::Empty; grid.palette.len()];
        let (feat, reqs) = extract_features_and_requests(&grid, &models, [0.0, 64.0, 0.0], 39.37);
        let atlas = build_texture_atlas(&reqs, &assets, 2);
        let chunks = mesh_world_chunks_with_chunk_size(&grid, &models, &atlas, &sign_atlas, &feat, [0.0, 64.0, 0.0], 39.37, 32);
        let sign_tris: Vec<_> = chunks[0]
            .solid_tris
            .iter()
            .filter(|t| t.material.as_ref() == "sign_atlas")
            .collect();
        assert!(!sign_tris.is_empty());
        let has_non_degenerate_uv = sign_tris.iter().any(|t| {
            (t.uv[0][0] - t.uv[1][0]).abs() > 1e-4 || (t.uv[0][1] - t.uv[2][1]).abs() > 1e-4
        });
        assert!(
            has_non_degenerate_uv,
            "Empty sign quads must map full wood base tile UVs rather than degenerate blank_uv point"
        );
    }

    #[test]
    fn test_mcgm_physics_step_cam_uses_origin_z_excess_not_raw_pos_double_sub() {
        use crate::glua::generate_mcgm_smooth_fall_autorun;

        let lua = generate_mcgm_physics_autorun();
        let fm_start = lua.find("hook.Add(\"FinishMove\"").expect("FinishMove");
        let cv_start = lua.find("MCGM_PhysicsStepCamera").expect("CalcView");
        let fm_body = &lua[fm_start..cv_start];
        let cv_body = &lua[cv_start..];

        assert!(
            !fm_body.contains("if onGround and prev.on_ground then"),
            "FinishMove must not subtract raw ground step dz into _mcgmCamStepZ (causes double-easing with SmoothViewOnStairs)"
        );
        assert!(
            cv_body.contains("_mcgmLastOriginZ") && cv_body.contains("GetCurrentViewOffset"),
            "CalcView must track incoming origin.z minus crouch view-offset delta"
        );
        assert!(
            !cv_body.contains("local curZ = ply:GetPos().z"),
            "CalcView must not compute step delta from raw ply:GetPos().z"
        );

        let sf = generate_mcgm_smooth_fall_autorun();
        assert!(
            !sf.contains("if onGround and wasGrounded then"),
            "mcgm_smooth_fall.lua must not double-subtract ground step dz on pos.z"
        );
    }

    #[test]
    fn test_main_chunk_qc_uses_opaque_not_mostlyopaque() {
        use crate::mesher::ChunkMesh;

        let tmp = std::env::temp_dir().join(format!("mcgm_test_qc_opaque_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let tri_cutout = MeshTri {
            material: "terrain_cutout_cull".into(),
            pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
            norm: [[0.0, 0.0, 1.0]; 3],
            uv: [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9]],
        };
        let cm = ChunkMesh {
            cx: 0,
            cz: 0,
            name: "chunk_0_0".to_string(),
            origin_source: [0.0, 0.0, 0.0],
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [39.37, 39.37, 39.37],
            solid_tris: vec![tri_cutout],
            translucent_tris: vec![],
            has_cutout: true,
        };
        let col = CollisionBox {
            box_coords: [0.0, 0.0, 0.0, 39.37, 39.37, 39.37],
            block_class: "full".into(),
            is_walkable: true,
            is_opaque_roof: true,
            surface_prop: SurfaceProp::Rock,
        };
        let art = emit_chunk_smds_and_qcs(&cm, &[col], &tmp, "custom_mc", 2048, "castle_map").unwrap();
        let qc = std::fs::read_to_string(&art.qc_path).unwrap();
        assert!(qc.contains("$opaque"), "Main solid+cutout chunk QC must use $opaque, got:\n{}", qc);
        assert!(!qc.contains("$mostlyopaque"), "Main solid+cutout chunk QC must not use $mostlyopaque");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_3d_leaves_use_culled_cutout_material_and_cross_grass_uses_nocull() {
        use crate::assets::{ModelElement, ModelFace};
        use crate::atlas::SignAtlas;
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        let air = make_state("minecraft:air", &[]);
        let leaves = make_state("minecraft:oak_leaves", &[]);
        let grass = make_state("minecraft:short_grass", &[]);

        let mut leaf_faces = HashMap::new();
        leaf_faces.insert(
            FaceDir::Up,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/oak_leaves".to_string(),
                cullface: Some(FaceDir::Up),
                rotation: 0,
                tint_index: Some(0),
            },
        );
        let leaf_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.0, 0.0, 0.0],
            to: [16.0, 16.0, 16.0],
            rotation: None,
            faces: leaf_faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);

        let mut grass_faces = HashMap::new();
        grass_faces.insert(
            FaceDir::North,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/short_grass".to_string(),
                cullface: None,
                rotation: 0,
                tint_index: Some(0),
            },
        );
        grass_faces.insert(
            FaceDir::South,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/short_grass".to_string(),
                cullface: None,
                rotation: 0,
                tint_index: Some(0),
            },
        );
        let grass_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.8, 0.0, 8.0],
            to: [15.2, 16.0, 8.0],
            rotation: None,
            faces: grass_faces,
            rot_x: 0,
            rot_y: 45,
            rot_z: 0,
            uvlock: false,
        }]);

        let mut assets = AssetIndex::empty();
        let leaf_img = ImageBuffer::from_pixel(16, 16, Rgba([200, 200, 200, 255]));
        let grass_img = ImageBuffer::from_pixel(16, 16, Rgba([200, 200, 200, 255]));
        assets.insert_entry("assets/minecraft/textures/block/oak_leaves.png", encode_png_bytes(&leaf_img));
        assets.insert_entry("assets/minecraft/textures/block/short_grass.png", encode_png_bytes(&grass_img));

        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 1, max_y: 64, max_z: 0 },
            palette: vec![air, leaves, grass],
            air_id: 0,
            blocks: vec![1, 2],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0, 0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let models = vec![ResolvedBlockModel::Empty, leaf_model, grass_model];
        let (feat, reqs) = extract_features_and_requests(&grid, &models, [0.0, 64.0, 0.0], 39.37);
        let atlas = build_texture_atlas(&reqs, &assets, 2);
        let sign_atlas = SignAtlas {
            image: image::RgbaImage::new(64, 64),
            rects: HashMap::new(),
        };
        let chunks = mesh_world_chunks_with_chunk_size(&grid, &models, &atlas, &sign_atlas, &feat, [0.0, 64.0, 0.0], 39.37, 32);
        let mats: Vec<&str> = chunks[0].solid_tris.iter().map(|t| t.material.as_ref()).collect();
        assert!(
            mats.contains(&"terrain_cutout_cull"),
            "3D leaves must use terrain_cutout_cull (no $nocull 1), got: {:?}",
            mats
        );
        assert!(
            mats.contains(&"terrain_cutout"),
            "Crossed short_grass must keep terrain_cutout ($nocull 1), got: {:?}",
            mats
        );
    }

    #[test]
    fn test_mc_water_brush_uses_block_scale_and_skybox_floor_is_nodraw() {
        let tmp = make_temp_test_dir("water_scale_skybox_floor");
        let vmf_path = tmp.join("water_sky.vmf");

        let col = CollisionBox {
            box_coords: [-39.37, -39.37, 0.0, 39.37, 39.37, 39.37],
            block_class: "full".into(),
            is_walkable: true,
            is_opaque_roof: true,
            surface_prop: SurfaceProp::Rock,
        };
        let art = CompiledChunkArtifact {
            chunk_name: "chunk_0_0".to_string(),
            origin_source: [0.0, 0.0, 0.0],
            qc_path: tmp.join("chunk_0_0.qc"),
            model_rel_path: "models/custom_mc/chunk_0_0.mdl".to_string(),
            translucent_qc_path: None,
            translucent_model_rel_path: None,
            extra_collision_models: Vec::new(),
            extra_collision_boxes: Vec::new(),
            overflow_boxes: vec![col.clone()],
            fallback_vmf_brushes: false,
        };
        build_vmf(&VmfBuildParams {
            output_vmf: &vmf_path,
            category: "custom_mc",
            bounds_min: [-100.0, -100.0, 0.0],
            bounds_max: [100.0, 100.0, 200.0],
            chunks: &[art],
            all_solid_boxes: &[col],
            ladder_boxes: &[],
            water_pools: &[[0.0, 0.0, 0.0, 39.37, 39.37, 35.0]],
            hazard_boxes: &[],
            core_occluders: &[],
            doors: &[],
            buttons: &[],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 48.0]],
            lighting_style: "minecraft",
            hq_light: false,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();

        let vmf = std::fs::read_to_string(&vmf_path).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        // 1. mc_water top side must use 39.37 / 64.0 = 0.615156 UV scale, not 0.25
        let water_pos = vmf.find("\"material\" \"custom_mc/mc_water\"").expect("mc_water side");
        let water_block = &vmf[water_pos..water_pos + 220.min(vmf.len() - water_pos)];
        assert!(
            water_block.contains("0.615156"),
            "mc_water brush must use 1-block UV scale (0.615156), got:\n{}",
            water_block
        );
        assert!(
            !water_block.contains("] 0.25"),
            "mc_water brush must not use hardcoded 0.25 UV scale"
        );

        // 2. Skybox floor (-264..-200) and any below-world skirt (z <= -10) must NOT emit TOOLSSKYBOX
        for chunk in vmf.split("side\n\t\t{").skip(1) {
            if chunk.contains("\"material\" \"TOOLS/TOOLSSKYBOX\"") {
                if let Some(plane_line) = chunk.lines().find(|l| l.contains("\"plane\"")) {
                    // Skybox floor sits at -200 / -264 (strictly below bounds_min[2] = 0.0)
                    let all_z_below_world = plane_line
                        .split('(')
                        .skip(1)
                        .filter_map(|p| p.split(')').next())
                        .all(|coords| {
                            coords
                                .split_whitespace()
                                .nth(2)
                                .and_then(|z| z.parse::<f64>().ok())
                                .map(|z| z < -1.0)
                                .unwrap_or(false)
                        });
                    assert!(
                        !all_z_below_world,
                        "Below-world skybox brush side must use TOOLS/TOOLSNODRAW, not TOOLSSKYBOX: {}",
                        plane_line
                    );
                }
            }
        }
    }

    #[test]
    fn test_waterlogged_blocks_and_seagrass_emit_water_faces_and_cull() {
        use crate::assets::{ModelElement, ModelFace};
        use crate::atlas::SignAtlas;
        use crate::collision::is_water_brush_eligible;
        use crate::mesher::{extract_features_and_requests, mesh_world_chunks_with_chunk_size};

        let seagrass = make_state("minecraft:seagrass", &[]);
        let kelp = make_state("minecraft:kelp", &[]);
        assert!(seagrass.waterlogged, "seagrass must be inherently waterlogged");
        assert!(kelp.waterlogged, "kelp must be inherently waterlogged");
        assert!(is_water_brush_eligible(&seagrass));

        let air = make_state("minecraft:air", &[]);
        let water = make_state("minecraft:water", &[("level", "0")]);
        let wl_stairs = make_state("minecraft:oak_stairs", &[("facing", "north"), ("waterlogged", "true")]);

        let mut assets = AssetIndex::empty();
        let img = ImageBuffer::from_pixel(16, 16, Rgba([160, 120, 70, 255]));
        assets.insert_entry("assets/minecraft/textures/block/oak_planks.png", encode_png_bytes(&img));
        assets.insert_entry("assets/minecraft/textures/block/water_still.png", encode_png_bytes(&img));
        assets.insert_entry("assets/minecraft/textures/block/water_flow.png", encode_png_bytes(&img));

        let mut stair_faces = HashMap::new();
        stair_faces.insert(
            FaceDir::Down,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: "minecraft:block/oak_planks".to_string(),
                cullface: None,
                rotation: 0,
                tint_index: None,
            },
        );
        let stair_model = ResolvedBlockModel::Elements(vec![ModelElement {
            from: [0.0, 0.0, 0.0],
            to: [16.0, 8.0, 16.0],
            rotation: None,
            faces: stair_faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }]);
        let water_model = assets.resolve_block_model(&water);
        let models = vec![ResolvedBlockModel::Empty, water_model, stair_model];

        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 1, max_y: 64, max_z: 0 },
            palette: vec![air, water, wl_stairs],
            air_id: 0,
            blocks: vec![1, 2],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0, 0],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let (feat, reqs) = extract_features_and_requests(&grid, &models, [0.0, 64.0, 0.0], 39.37);
        let atlas = build_texture_atlas(&reqs, &assets, 2);
        let sign_atlas = SignAtlas {
            image: image::RgbaImage::new(64, 64),
            rects: HashMap::new(),
        };
        let chunks = mesh_world_chunks_with_chunk_size(&grid, &models, &atlas, &sign_atlas, &feat, [0.0, 64.0, 0.0], 39.37, 32);

        // Waterlogged stair at wx=1 (Source X in 39.37..78.74) must emit translucent water faces
        let stair_water_tris: Vec<_> = chunks[0]
            .translucent_tris
            .iter()
            .filter(|t| t.material.as_ref() == "mc_water_flow" && t.pos[0][0] >= 39.36)
            .collect();
        assert!(
            !stair_water_tris.is_empty(),
            "Waterlogged stairs must emit translucent water faces"
        );
        // Shared X=39.37 boundary between water (wx=0) and waterlogged_stairs (wx=1) must be culled
        let seam_tris: Vec<_> = chunks[0]
            .translucent_tris
            .iter()
            .filter(|t| {
                t.material.as_ref() == "mc_water_flow"
                    && t.pos.iter().all(|p| (p[0] - 39.37).abs() < 0.1)
            })
            .collect();
        assert!(
            seam_tris.is_empty(),
            "Shared water face between water and waterlogged block must be culled, got {} tris",
            seam_tris.len()
        );
    }

    #[test]
    fn test_ladder_collision_is_thin_and_aligned_to_facing() {
        use crate::collision::build_world_collision_with_chunk_size;

        let air = make_state("minecraft:air", &[]);
        let lad_n = make_state("minecraft:ladder", &[("facing", "north")]);
        let lad_s = make_state("minecraft:ladder", &[("facing", "south")]);
        let lad_e = make_state("minecraft:ladder", &[("facing", "east")]);
        let lad_w = make_state("minecraft:ladder", &[("facing", "west")]);

        let coords = WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 9, max_y: 64, max_z: 0 };
        let mut blocks = vec![0u16; 10];
        blocks[0] = 1; // north at wx=0
        blocks[3] = 2; // south at wx=3
        blocks[6] = 3; // east at wx=6
        blocks[9] = 4; // west at wx=9

        let grid = VoxelGrid {
            coords,
            palette: vec![air, lad_n, lad_s, lad_e, lad_w],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 10],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision_with_chunk_size(&grid, [0.0, 64.0, 0.0], 39.37, 32);
        assert_eq!(col.ladder_boxes.len(), 4);
        for lb in &col.ladder_boxes {
            let sx = (lb[3] - lb[0]).abs();
            let sy = (lb[4] - lb[1]).abs();
            let min_horiz = sx.min(sy);
            assert!(
                min_horiz <= 0.20 * 39.37,
                "Ladder brush must be thin against wall (<= 0.20 blocks), got {}x{} HU",
                sx,
                sy
            );
        }
        let climb_zones: Vec<_> = col
            .physics_zones
            .iter()
            .filter(|z| z.kind == "ladder")
            .collect();
        assert_eq!(climb_zones.len(), 4);
        for cz in climb_zones {
            let sx = (cz.max[0] - cz.min[0]).abs();
            let sy = (cz.max[1] - cz.min[1]).abs();
            assert!(
                sx.min(sy) <= 0.25 * 39.37,
                "Ladder climb physics zone must be thin against wall, got {}x{} HU",
                sx,
                sy
            );
        }
    }

    #[test]
    fn test_water_flow_tile_preserves_16px_block_texel_density() {
        use crate::compiler::export_animated_water_textures;

        let tmp = make_temp_test_dir("water_flow_density");
        let mut assets = AssetIndex::empty();
        // Vanilla water_flow is 32x32 (2x2 tiles of 16x16). Put bright pixels in top-left 16x16, dark elsewhere.
        let mut flow_img = ImageBuffer::from_pixel(32, 32, Rgba([40, 40, 40, 255]));
        for y in 0..16u32 {
            for x in 0..16u32 {
                flow_img.put_pixel(x, y, Rgba([220, 220, 220, 255]));
            }
        }
        assets.insert_entry("assets/minecraft/textures/block/water_flow.png", encode_png_bytes(&flow_img));
        assets.insert_entry(
            "assets/minecraft/textures/block/water_still.png",
            encode_png_bytes(&ImageBuffer::from_pixel(16, 16, Rgba([220, 220, 220, 255]))),
        );

        let (_still_p, flow_p) = export_animated_water_textures(&assets, &tmp).unwrap();
        let out_flow = image::open(&flow_p).unwrap().to_rgba8();
        let _ = std::fs::remove_dir_all(&tmp);

        // If top-left 16x16 was scaled to 64x64, bottom-right pixel (48, 48) is bright blue (b > 150), not dark (b < 60)
        let px = out_flow.get_pixel(48, 48);
        assert!(
            px[2] > 150,
            "32x32 water_flow must crop single 16x16 tile density before upscaling to 64x64, got {:?}",
            px
        );
    }

    #[test]
    fn test_flowing_fluid_slope_seams_side_uv_and_lava_crop() {
        use crate::atlas::{build_texture_atlas, collect_animated_materials, RenderBucket, SignAtlas, TexRequestKey};
        use crate::mesher::{stream_world_chunks_with_animated, ExtractedFeatures};
        use image::{ImageBuffer, Rgba, RgbaImage};
        use std::collections::HashSet;

        let mut assets = AssetIndex::empty();
        // 32x64 (2 frames of 32x32) lava_flow: top-left 16x16 quadrant is orange [255, 100, 0, 255],
        // other quadrants are blue [0, 0, 255, 255].
        let mut lava_strip = ImageBuffer::from_pixel(32, 64, Rgba([0, 0, 255, 255]));
        for frame in 0..2u32 {
            for y in 0..16u32 {
                for x in 0..16u32 {
                    lava_strip.put_pixel(x, frame * 32 + y, Rgba([255, 100, 0, 255]));
                }
            }
        }
        assets.insert_entry("assets/minecraft/textures/block/lava_flow.png", encode_png_bytes(&lava_strip));
        assets.insert_entry(
            "assets/minecraft/textures/block/lava_still.png",
            encode_png_bytes(&ImageBuffer::from_pixel(16, 32, Rgba([255, 120, 0, 255]))),
        );
        assets.insert_entry(
            "assets/minecraft/textures/block/water_flow.png",
            encode_png_bytes(&ImageBuffer::from_pixel(32, 32, Rgba([200, 200, 200, 255]))),
        );
        assets.insert_entry(
            "assets/minecraft/textures/block/water_still.png",
            encode_png_bytes(&ImageBuffer::from_pixel(16, 16, Rgba([200, 200, 200, 255]))),
        );

        let mut anim_reqs = HashSet::new();
        anim_reqs.insert(TexRequestKey {
            tex_ref: "minecraft:block/lava_flow".to_string(),
            tint_rgb: None,
        });
        let (anim_lookup, anim_entries) = collect_animated_materials(&anim_reqs, &assets);
        assert_eq!(anim_entries.len(), 1, "lava_flow must emit animated entry");
        let lava_px = anim_entries[0].frames[0].get_pixel(48, 48);
        assert!(
            lava_px[0] > 200 && lava_px[2] < 50,
            "32x32 lava_flow frame must crop 16x16 top-left tile so 1x1 block pixel scale matches 16px blocks, got {:?}",
            lava_px
        );

        let air = make_state("minecraft:air", &[]);
        let lava_src = make_state("minecraft:lava", &[("level", "8")]);
        let lava_low = make_state("minecraft:lava", &[("level", "3")]);
        let water_src = make_state("minecraft:water", &[("level", "0")]);
        let water_flow = make_state("minecraft:water", &[("level", "2")]);

        // Verify fluid model side face UV is not hardcoded to full 0..16 regardless of partial height
        if let ResolvedBlockModel::Elements(els) = assets.resolve_block_model(&lava_low) {
            let side_uv = els[0].faces.get(&FaceDir::North).unwrap().uv.unwrap();
            assert!(
                (side_uv[3] - side_uv[1]).abs() < 12.0,
                "Partial-height fluid side UV must scale proportionally to height, got {:?}",
                side_uv
            );
        } else {
            panic!("lava_low must resolve to elements");
        }

        // World 4x2x1 (x=0..3, y=64..65, z=0):
        // (0, 64, 0) = lava(8), (0, 65, 0) = lava(8) -> tall column at x=0 (top=65.0)
        // (1, 64, 0) = lava(3) -> lower flowing lava at x=1 (top < 64.7), air at (1,65,0)
        // (2, 64, 0) = water(0) -> source water at x=2
        // (3, 64, 0) = water(2) -> flowing water at x=3 next to air at x=4 -> must slope!
        let mut req_keys = HashSet::new();
        req_keys.insert(TexRequestKey {
            tex_ref: "minecraft:block/lava_still".to_string(),
            tint_rgb: None,
        });
        let atlas = build_texture_atlas(&req_keys, &assets, 2);
        let sign_atlas = SignAtlas {
            image: RgbaImage::new(64, 64),
            rects: HashMap::new(),
        };
        let mut animated_mats = anim_lookup;
        animated_mats.insert(
            TexRequestKey {
                tex_ref: "minecraft:block/lava_still".to_string(),
                tint_rgb: None,
            },
            ("anim_lava_still".to_string(), RenderBucket::TerrainOpaque),
        );

        let coords = WorldCoords { min_x: 0, min_y: 64, min_z: 0, max_x: 3, max_y: 65, max_z: 0 };
        let mut blocks = vec![0u16; 8];
        // idx = (gy * sz + gz) * sx + gx; sy=2, sz=1, sx=4
        blocks[0] = 1; // (0, 64, 0) lava(8)
        blocks[4] = 1; // (0, 65, 0) lava(8)
        blocks[1] = 2; // (1, 64, 0) lava(3)
        blocks[2] = 3; // (2, 64, 0) water(0)
        blocks[3] = 4; // (3, 64, 0) water(2)

        let palette = vec![air, lava_src, lava_low, water_src, water_flow];
        let models: Vec<_> = palette
            .iter()
            .map(|bs| {
                if bs.is_air {
                    ResolvedBlockModel::Empty
                } else {
                    assets.resolve_block_model(bs)
                }
            })
            .collect();
        let grid = VoxelGrid {
            coords,
            palette,
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 8],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let features = ExtractedFeatures {
            lights: Vec::new(),
            doors: Vec::new(),
            door_block_names: HashSet::new(),
            door_voxels: HashSet::new(),
            flat_water_voxels: HashSet::new(),
            buttons: Vec::new(),
            spawn_points_mc: Vec::new(),
            skip_mesh_bits: Vec::new(),
        };
        let chunks = stream_world_chunks_with_animated(
            &grid,
            &models,
            &atlas,
            &sign_atlas,
            &features,
            [0.0, 64.0, 0.0],
            39.37,
            32,
            &animated_mats,
            Ok,
        )
        .unwrap();

        let scale = 39.37f32;
        // 1) Lava height-step seam at x = 1 block = 39.37 HU between lava(8+above -> 1.0) and lava(3 -> <0.7)
        let has_lava_step_wall = chunks[0].solid_tris.iter().any(|t| {
            let x_match = t.pos.iter().all(|p| (p[0] - scale).abs() < 0.1);
            let z_vals: Vec<f32> = t.pos.iter().map(|p| p[2]).collect();
            let z_min = z_vals.iter().cloned().fold(f32::INFINITY, f32::min);
            let z_max = z_vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            x_match && z_max > 0.9 * scale && z_min < 0.85 * scale
        });
        assert!(
            has_lava_step_wall,
            "Tall lava column (y=64 with lava above) must emit connecting step face to adjacent lower lava(3) instead of leaving a black hole"
        );

        // 2) Flowing water at x=3 (x in 3*scale..4*scale) top face must be sloped (not flat slab)
        let water_top_zs: Vec<f32> = chunks[0]
            .translucent_tris
            .iter()
            .filter(|t| {
                let in_x3 = t.pos.iter().all(|p| p[0] >= 3.0 * scale - 0.1 && p[0] <= 4.0 * scale + 0.1);
                let is_top = t.norm.iter().all(|n| n[2] > 0.5);
                in_x3 && is_top
            })
            .flat_map(|t| t.pos.iter().map(|p| p[2]))
            .collect();
        assert!(!water_top_zs.is_empty(), "Expected top tris for flowing water");
        let w_min = water_top_zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let w_max = water_top_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            w_max - w_min > 2.0,
            "Flowing water top surface must slope smoothly between higher neighbor and air, got z span {}..{}",
            w_min,
            w_max
        );

        // 3) Side faces on low water (east face at x = 4*scale) must not squish full 0..1 V span into tiny height
        let low_water_side_tris: Vec<_> = chunks[0]
            .translucent_tris
            .iter()
            .filter(|t| {
                t.pos.iter().all(|p| (p[0] - 4.0 * scale).abs() < 0.1)
                    && t.norm.iter().all(|n| n[0] > 0.5)
            })
            .collect();
        assert!(!low_water_side_tris.is_empty(), "Expected outer side face on flowing water");
        for t in low_water_side_tris {
            let v_min = t.uv.iter().map(|uv| uv[1]).fold(f32::INFINITY, f32::min);
            let v_max = t.uv.iter().map(|uv| uv[1]).fold(f32::NEG_INFINITY, f32::max);
            assert!(
                (v_max - v_min) < 0.85,
                "Low flowing water side UV must preserve 1:1 vertical texel scale instead of 0..1 squishing, got v span {}..{}",
                v_min,
                v_max
            );
        }
    }

    #[test]
    fn test_gui_native_windows_controls_and_system_theme() {
        let gui_src = include_str!("gui.rs");
        let pal = light_palette();
        assert_eq!(
            pal.canvas_bg,
            0x00F0_F0F0,
            "Native Windows dialog background must match COLOR_3DFACE rgb(240,240,240)"
        );
        assert!(
            !gui_src.contains("BS_OWNERDRAW"),
            "GUI must use genuine native Win32 ComCtl32 v6 controls instead of BS_OWNERDRAW"
        );
        assert!(
            gui_src.contains("BS_DEFPUSHBUTTON")
                && gui_src.contains("BS_PUSHBUTTON")
                && gui_src.contains("BS_AUTOCHECKBOX")
                && gui_src.contains("BS_GROUPBOX"),
            "GUI must use native BS_DEFPUSHBUTTON, BS_PUSHBUTTON, BS_AUTOCHECKBOX, and BS_GROUPBOX"
        );
        assert!(
            !gui_src.contains("subclass_control(h_prog"),
            "Progress bar must use native ComCtl32 msctls_progress32 visual styles without custom RoundRect subclassing"
        );
    }

    #[test]
    fn test_glua_sprint_faster_and_no_sprint_jump_bhop_boost() {
        let lua = generate_mcgm_physics_autorun();
        assert!(
            !lua.contains("local sprintSpeed = 5.612 * S"),
            "MC sprint speed must be faster than old 5.612 * S"
        );
        assert!(
            lua.contains("local sprintSpeed = 7.2 * S"),
            "MC sprint speed should be raised to 7.2 * S"
        );
        assert!(
            !lua.contains("local boost = 42.0"),
            "Source-like +42.0 sprint-jump forward bhop boost must be removed"
        );
        assert!(
            !lua.contains("ctx.sprintSpeed * 1.25"),
            "Source-like sprint-jump 1.25x bhop speed cap must be removed"
        );
        assert!(
            lua.contains("ctx.sprintSpeed * 0.88"),
            "Sprint-jump takeoff/airborne speed must be capped below sprint speed to prevent Source bunny-hopping"
        );
    }

    #[test]
    fn test_gui_outdir_renamed_and_no_clipchildren_black_groupbox_bug() {
        assert_eq!(tr_text(UiLang::Ru, "lbl_outdir"), "Куда сохранить:");
        assert_eq!(tr_text(UiLang::En, "lbl_outdir"), "Save to:");
        for id in [crate::gui::ID_LBL_OUTDIR, ID_EDIT_OUTDIR, crate::gui::ID_BTN_BROWSE_OUT] {
            assert!(
                !is_advanced_only_control(id),
                "Save-to control {} must be in Simple mode",
                id
            );
        }
        let gui_src = include_str!("gui.rs");
        assert!(
            !gui_src.contains("WS_CLIPCHILDREN"),
            "Main window must not use WS_CLIPCHILDREN so BS_GROUPBOX interiors repaint cleanly without black gaps"
        );
        assert!(
            gui_src.contains("RedrawWindow"),
            "Resizing or toggling panels must call RedrawWindow to avoid ghost control trails"
        );
    }

    #[test]
    fn test_water_animation_uses_multiframe_mc_framerate_not_texture_scroll() {
        use crate::compiler::{compile_water_materials, export_animated_water_textures, ToolPaths};
        use image::{ImageBuffer, Rgba};

        let tmp = make_temp_test_dir("water_mc_anim");
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&addon_dir).unwrap();

        let mut assets = AssetIndex::empty();
        // 16x32 water_still (2 distinct frames) + frametime 2 => 10.0 fps
        let mut still_strip = ImageBuffer::from_pixel(16, 32, Rgba([180, 180, 180, 255]));
        for y in 16..32u32 {
            for x in 0..16u32 {
                still_strip.put_pixel(x, y, Rgba([240, 240, 240, 255]));
            }
        }
        assets.insert_entry(
            "assets/minecraft/textures/block/water_still.png",
            encode_png_bytes(&still_strip),
        );
        assets.insert_entry(
            "assets/minecraft/textures/block/water_still.png.mcmeta",
            br#"{"animation":{"frametime":2}}"#.to_vec(),
        );

        // 32x64 water_flow (2 distinct 32x32 frames, cropped 16x16 per frame) + frametime 1 => 20.0 fps
        let mut flow_strip = ImageBuffer::from_pixel(32, 64, Rgba([160, 160, 160, 255]));
        for y in 0..16u32 {
            for x in 0..16u32 {
                flow_strip.put_pixel(x, 32 + y, Rgba([250, 250, 250, 255]));
            }
        }
        assets.insert_entry(
            "assets/minecraft/textures/block/water_flow.png",
            encode_png_bytes(&flow_strip),
        );
        assets.insert_entry(
            "assets/minecraft/textures/block/water_flow.png.mcmeta",
            br#"{"animation":{"frametime":1}}"#.to_vec(),
        );

        let (still_png, flow_png) = export_animated_water_textures(&assets, &work_dir).unwrap();
        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            "@echo off\r\nfor %%F in (%2) do echo DUMMY > \"%~4\\%%~nF.vtf\"\r\n",
        )
        .unwrap();
        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };
        compile_water_materials(&tools, "custom_mc", &still_png, &flow_png, &addon_dir, false).unwrap();

        let still_vtf = std::fs::read(addon_dir.join("materials/custom_mc/mc_water.vtf")).unwrap();
        let flow_vtf = std::fs::read(addon_dir.join("materials/models/custom_mc/mc_water_flow.vtf")).unwrap();
        assert!(still_vtf.len() >= 80 && &still_vtf[0..4] == b"VTF\0", "mc_water.vtf must be real multi-frame VTF");
        assert!(flow_vtf.len() >= 80 && &flow_vtf[0..4] == b"VTF\0", "mc_water_flow.vtf must be real multi-frame VTF");
        assert_eq!(u16::from_le_bytes([still_vtf[24], still_vtf[25]]), 2, "mc_water.vtf must contain 2 frames");
        assert_eq!(u16::from_le_bytes([flow_vtf[24], flow_vtf[25]]), 2, "mc_water_flow.vtf must contain 2 frames");

        let brush_vmt = std::fs::read_to_string(addon_dir.join("materials/custom_mc/mc_water.vmt")).unwrap();
        let beneath_vmt = std::fs::read_to_string(addon_dir.join("materials/custom_mc/mc_water_beneath.vmt")).unwrap();
        let flow_vmt = std::fs::read_to_string(addon_dir.join("materials/models/custom_mc/mc_water_flow.vmt")).unwrap();
        for (name, vmt) in [("mc_water", &brush_vmt), ("mc_water_beneath", &beneath_vmt), ("mc_water_flow", &flow_vmt)] {
            assert!(
                !vmt.contains("TextureScroll"),
                "{}.vmt must not use 60fps TextureScroll",
                name
            );
            assert!(
                vmt.contains("\"AnimatedTexture\""),
                "{}.vmt must use Minecraft flipbook AnimatedTexture proxy",
                name
            );
        }
        assert!(brush_vmt.contains("\"animatedTextureFrameRate\" \"10.00\""));
        assert!(flow_vmt.contains("\"animatedTextureFrameRate\" \"20.00\""));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_no_import_to_gmod_preserves_preexisting_models_on_success_error_and_cancel() {
        use crate::compiler::{
            compile_chunk_models_cached, stage_cached_models_to_game_dir, StagedModelGuard,
            ToolPaths,
        };

        let tmp = make_temp_test_dir("no_import_models");
        let addon_dir = tmp.join("addon");
        let game_dir = tmp.join("game");
        let work_dir = tmp.join("work");
        let smd_dir = work_dir.join("smd");
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&smd_dir).unwrap();

        let mock_studiomdl = tmp.join("mock_studiomdl.cmd");
        std::fs::write(
            &mock_studiomdl,
            "@echo off\r\n\
            setlocal enabledelayedexpansion\r\n\
            set \"GAME=\"\r\n\
            set \"NEXT_GAME=\"\r\n\
            set \"QC=\"\r\n\
            for %%A in (%*) do (\r\n\
                if defined NEXT_GAME (\r\n\
                    set \"GAME=%%~A\"\r\n\
                    set \"NEXT_GAME=\"\r\n\
                ) else if \"%%~A\"==\"-game\" (\r\n\
                    set \"NEXT_GAME=1\"\r\n\
                ) else (\r\n\
                    set \"QC=%%~nA\"\r\n\
                )\r\n\
            )\r\n\
            if \"!QC!\"==\"fail_chunk\" exit /b 1\r\n\
            mkdir \"!GAME!\\models\\custom_mc\\gm_map\" 2>nul\r\n\
            echo NEW_MDL_!QC!>\"!GAME!\\models\\custom_mc\\gm_map\\!QC!.mdl\"\r\n\
            echo NEW_VVD_!QC!>\"!GAME!\\models\\custom_mc\\gm_map\\!QC!.vvd\"\r\n\
            echo NEW_DX90_!QC!>\"!GAME!\\models\\custom_mc\\gm_map\\!QC!.dx90.vtx\"\r\n\
            echo NEW_VTX_!QC!>\"!GAME!\\models\\custom_mc\\gm_map\\!QC!.vtx\"\r\n\
            echo NEW_PHY_!QC!>\"!GAME!\\models\\custom_mc\\gm_map\\!QC!.phy\"\r\n\
            exit /b 0\r\n",
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: mock_studiomdl,
            vtfcmd: std::path::PathBuf::new(),
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: game_dir.clone(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };

        let existing_rel = "models/custom_mc/gm_map/chunk_0_0.mdl";
        let temp_rel = "models/custom_mc/gm_map/chunk_1_0.mdl";
        let game_model_dir = game_dir.join("models/custom_mc/gm_map");
        std::fs::create_dir_all(&game_model_dir).unwrap();

        let originals: [(&str, &[u8]); 5] = [
            ("chunk_0_0.mdl", b"ORIG_MDL_BYTES\x00\x01"),
            ("chunk_0_0.vvd", b"ORIG_VVD_BYTES\x02\x03"),
            ("chunk_0_0.vtx", b"ORIG_VTX_BYTES\x04\x05"),
            ("chunk_0_0.dx90.vtx", b"ORIG_DX90_VTX_BYTES\x06\x07"),
            ("chunk_0_0.phy", b"ORIG_PHY_BYTES\x08\x09"),
        ];
        for (fname, data) in &originals {
            std::fs::write(game_model_dir.join(fname), data).unwrap();
        }

        let qc0 = smd_dir.join("chunk_0_0.qc");
        let qc1 = smd_dir.join("chunk_1_0.qc");
        let smd0 = smd_dir.join("chunk_0_0.smd");
        let smd1 = smd_dir.join("chunk_1_0.smd");
        std::fs::write(&smd0, "version 1\nend\n").unwrap();
        std::fs::write(&smd1, "version 1\nend\n").unwrap();
        std::fs::write(&qc0, "$modelname \"custom_mc/gm_map/chunk_0_0.mdl\"\n$body \"b\" \"chunk_0_0.smd\"\n").unwrap();
        std::fs::write(&qc1, "$modelname \"custom_mc/gm_map/chunk_1_0.mdl\"\n$body \"b\" \"chunk_1_0.smd\"\n").unwrap();

        let mut chunks = vec![
            CompiledChunkArtifact {
                chunk_name: "chunk_0_0".to_string(),
                origin_source: [0.0; 3],
                qc_path: qc0.clone(),
                model_rel_path: existing_rel.to_string(),
                translucent_qc_path: None,
                translucent_model_rel_path: None,
                extra_collision_models: Vec::new(),
                extra_collision_boxes: Vec::new(),
                overflow_boxes: Vec::new(),
                fallback_vmf_brushes: false,
            },
            CompiledChunkArtifact {
                chunk_name: "chunk_1_0".to_string(),
                origin_source: [0.0; 3],
                qc_path: qc1.clone(),
                model_rel_path: temp_rel.to_string(),
                translucent_qc_path: None,
                translucent_model_rel_path: None,
                extra_collision_models: Vec::new(),
                extra_collision_boxes: Vec::new(),
                overflow_boxes: Vec::new(),
                fallback_vmf_brushes: false,
            },
        ];

        // 1. Direct uncached compile with import_to_gmod = false: pre-existing files must stay byte-identical,
        // newly compiled chunk_1_0 files must be copied to addon_dir and removed from game_dir.
        compile_chunk_models_cached(&tools, &mut chunks, &addon_dir, false, 1, false, |_, _, _, _| {})
            .unwrap();

        for (fname, data) in &originals {
            assert_eq!(
                std::fs::read(game_model_dir.join(fname)).unwrap(),
                *data,
                "pre-existing {} must stay byte-for-byte intact",
                fname
            );
            assert!(
                addon_dir.join("models/custom_mc/gm_map").join(fname).is_file(),
                "addon_dir must receive compiled {}",
                fname
            );
        }
        for ext in [".mdl", ".vvd", ".vtx", ".dx90.vtx", ".phy"] {
            assert!(
                !game_model_dir.join(format!("chunk_1_0{}", ext)).exists(),
                "newly created chunk_1_0{} must be removed from game_dir",
                ext
            );
            assert!(
                addon_dir.join(format!("models/custom_mc/gm_map/chunk_1_0{}", ext)).is_file(),
                "addon_dir must receive chunk_1_0{}",
                ext
            );
        }

        // 2. Pipeline-style StagedModelGuard around cached staging + simulated failure/cancellation
        let rel_list = vec![existing_rel.to_string(), temp_rel.to_string()];
        {
            let _guard = StagedModelGuard::prepare(&game_dir, &rel_list, &work_dir).unwrap();
            stage_cached_models_to_game_dir(&addon_dir, &game_dir, &chunks).unwrap();
            assert!(
                game_model_dir.join("chunk_1_0.mdl").is_file(),
                "cached model should be temporarily staged in game_dir during VBSP"
            );
            // Guard drops here (simulating success, error, or cancellation)
        }
        for (fname, data) in &originals {
            assert_eq!(
                std::fs::read(game_model_dir.join(fname)).unwrap(),
                *data,
                "after guard drop, pre-existing {} must match original bytes",
                fname
            );
        }
        for ext in [".mdl", ".vvd", ".vtx", ".dx90.vtx", ".phy"] {
            assert!(
                !game_model_dir.join(format!("chunk_1_0{}", ext)).exists(),
                "after guard drop, temporary chunk_1_0{} must be deleted from game_dir",
                ext
            );
        }

        // 3. Cached compile_chunk_models_cached with reuse_existing = true and import_to_gmod = false
        compile_chunk_models_cached(&tools, &mut chunks, &addon_dir, false, 1, true, |_, _, _, _| {})
            .unwrap();
        for (fname, data) in &originals {
            assert_eq!(std::fs::read(game_model_dir.join(fname)).unwrap(), *data);
        }
        assert!(!game_model_dir.join("chunk_1_0.mdl").exists());

        // 4. StudioMDL error on a subsequent chunk must still restore pre-existing game files
        let fail_qc = smd_dir.join("fail_chunk.qc");
        std::fs::write(&fail_qc, "$modelname \"custom_mc/gm_map/fail_chunk.mdl\"\n").unwrap();
        let mut fail_chunks = vec![
            chunks[0].clone(),
            CompiledChunkArtifact {
                chunk_name: "fail_chunk".to_string(),
                origin_source: [0.0; 3],
                qc_path: fail_qc,
                model_rel_path: "models/custom_mc/gm_map/fail_chunk.mdl".to_string(),
                translucent_qc_path: None,
                translucent_model_rel_path: None,
                extra_collision_models: Vec::new(),
                extra_collision_boxes: Vec::new(),
                overflow_boxes: Vec::new(),
                fallback_vmf_brushes: false,
            },
        ];
        let err_res = compile_chunk_models_cached(
            &tools,
            &mut fail_chunks,
            &addon_dir,
            false,
            1,
            false,
            |_, _, _, _| {},
        );
        assert!(err_res.is_err(), "fail_chunk must fail StudioMDL");
        for (fname, data) in &originals {
            assert_eq!(std::fs::read(game_model_dir.join(fname)).unwrap(), *data);
        }
        assert!(!game_model_dir.join("fail_chunk.mdl").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_gma_preserves_previous_archive_on_write_failure_and_reports_summary() {
        use crate::compiler::{create_gma_addon, pack_gma_v3_native_with_hook, ToolPaths};

        let tmp = make_temp_test_dir("gma_atomic_preserve");
        let addon_dir = tmp.join("addon");
        std::fs::create_dir_all(addon_dir.join("maps")).unwrap();
        std::fs::write(addon_dir.join("maps/gm_demo.bsp"), b"VBSP_DATA").unwrap();

        let out_gma = tmp.join("gm_demo.gma");
        let prev_bytes = b"GMAD_PREVIOUS_ARCHIVE_KEEP_ME_INTACT";
        std::fs::write(&out_gma, prev_bytes).unwrap();

        // 1. Native packer simulated failure AFTER writing header & file list
        let mut saw_temp_file = false;
        let err = pack_gma_v3_native_with_hook(
            &addon_dir,
            &out_gma,
            "MCGM - gm_demo",
            "{}",
            |tmp_path| {
                saw_temp_file = tmp_path != out_gma
                    && tmp_path.is_file()
                    && tmp_path.metadata().map(|m| m.len() > 0).unwrap_or(false);
                Err("simulated I/O failure mid-write".to_string())
            },
        )
        .unwrap_err();
        assert!(err.contains("simulated I/O failure"));
        assert!(saw_temp_file, "packer must write to a separate temp file first");
        assert_eq!(
            std::fs::read(&out_gma).unwrap(),
            prev_bytes,
            "previous .gma must remain byte-for-byte intact on write failure"
        );
        let leftover_tmps: Vec<_> = std::fs::read_dir(&tmp)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp.gma"))
            .collect();
        assert!(leftover_tmps.is_empty(), "temp .gma file must be cleaned up on error");

        // 2. Failing gmad.exe writing partial output + failing native fallback (missing addon_dir)
        let bad_gmad = tmp.join("bad_gmad.cmd");
        std::fs::write(
            &bad_gmad,
            "@echo off\r\necho PARTIAL_GARBAGE>\"%~5\"\r\nexit /b 9\r\n",
        )
        .unwrap();
        let tools_bad = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: std::path::PathBuf::new(),
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: Some(bad_gmad),
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };
        let missing_addon = tmp.join("nonexistent_addon");
        let mut logs = Vec::new();
        let res = create_gma_addon(&tools_bad, &missing_addon, &out_gma, "gm_demo", |m| {
            logs.push(m.to_string());
        });
        assert!(res.is_err());
        assert!(
            logs.iter().any(|l| l.contains("falling back to native GMA v3 packer")),
            "must log gmad -> native fallback"
        );
        assert_eq!(
            std::fs::read(&out_gma).unwrap(),
            prev_bytes,
            "previous .gma must remain intact when both gmad and native fail"
        );

        // 3. Failing gmad.exe falls back to native packer on valid addon_dir and atomically replaces out_gma
        let ok_res = create_gma_addon(&tools_bad, &addon_dir, &out_gma, "gm_demo", |_| {});
        assert!(ok_res.is_ok());
        let new_bytes = std::fs::read(&out_gma).unwrap();
        assert_eq!(&new_bytes[0..4], b"GMAD");
        assert_ne!(new_bytes, prev_bytes);

        // 4. GUI summary explicitly reports when BSP succeeded but GMA failed
        let summary_gma_err = crate::PipelineSummary {
            bsp_path: std::path::PathBuf::from("output/gm_demo.bsp"),
            gma_path: None,
            gma_error: Some("disk full".to_string()),
            chunk_count: 4,
            door_count: 1,
            total_secs: 1.5,
            stage_timings: vec![],
        };
        let ru = format_done_summary(UiLang::Ru, &summary_gma_err);
        let en = format_done_summary(UiLang::En, &summary_gma_err);
        assert!(ru.contains("BSP") && ru.contains("GMA") && ru.contains("disk full"));
        assert!(en.contains("BSP") && en.contains("GMA") && en.contains("disk full"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_two_maps_same_category_do_not_overwrite_materials_or_signs() {
        use crate::compiler::{compile_materials, scoped_map_category, ToolPaths};

        let tmp = make_temp_test_dir("two_maps_category_isolation");
        let game_dir = tmp.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            "@echo off\r\ncopy /y \"%~2\" \"%~4\\%~n2.vtf\" >nul\r\n",
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: game_dir.clone(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };

        let cat_a = scoped_map_category("custom_mc", "gm_map_alpha");
        let cat_b = scoped_map_category("custom_mc", "gm_map_beta");
        assert_ne!(cat_a, cat_b);

        let work_a = tmp.join("work_a");
        let work_b = tmp.join("work_b");
        let addon_a = tmp.join("addon_a");
        let addon_b = tmp.join("addon_b");
        std::fs::create_dir_all(&work_a).unwrap();
        std::fs::create_dir_all(&work_b).unwrap();

        let atlas_a = work_a.join("atlas_aaaa1111.png");
        let sign_a = work_a.join("sign_atlas.png");
        crate::atlas::save_png_fast(&ImageBuffer::from_pixel(16, 16, Rgba([220, 20, 20, 255])), &atlas_a).unwrap();
        crate::atlas::save_png_fast(&ImageBuffer::from_pixel(16, 16, Rgba([250, 240, 10, 255])), &sign_a).unwrap();

        let atlas_b = work_b.join("atlas_bbbb2222.png");
        let sign_b = work_b.join("sign_atlas.png");
        crate::atlas::save_png_fast(&ImageBuffer::from_pixel(16, 16, Rgba([20, 40, 220, 255])), &atlas_b).unwrap();
        crate::atlas::save_png_fast(&ImageBuffer::from_pixel(16, 16, Rgba([10, 220, 240, 255])), &sign_b).unwrap();

        compile_materials(&tools, &cat_a, &atlas_a, &sign_a, &HashMap::new(), &[], &addon_a, true, false).unwrap();
        compile_materials(&tools, &cat_b, &atlas_b, &sign_b, &HashMap::new(), &[], &addon_b, true, false).unwrap();

        // Both maps' VMTs and sign atlases coexist in game_dir without overwriting each other
        let sign_vtf_a = game_dir.join("materials/models").join(&cat_a).join("sign_atlas.vtf");
        let sign_vtf_b = game_dir.join("materials/models").join(&cat_b).join("sign_atlas.vtf");
        assert_eq!(std::fs::read(&sign_vtf_a).unwrap(), std::fs::read(&sign_a).unwrap());
        assert_eq!(std::fs::read(&sign_vtf_b).unwrap(), std::fs::read(&sign_b).unwrap());

        let vmt_a = std::fs::read_to_string(
            game_dir.join("materials/models").join(&cat_a).join("terrain_opaque.vmt"),
        )
        .unwrap();
        let vmt_b = std::fs::read_to_string(
            game_dir.join("materials/models").join(&cat_b).join("terrain_opaque.vmt"),
        )
        .unwrap();
        assert!(vmt_a.contains("models/custom_mc/gm_map_alpha/atlas_aaaa1111"));
        assert!(vmt_b.contains("models/custom_mc/gm_map_beta/atlas_bbbb2222"));

        // Models and VMF reference their own scoped category paths
        let cm = crate::mesher::ChunkMesh {
            cx: 0,
            cz: 0,
            name: "chunk_0_0".to_string(),
            origin_source: [0.0; 3],
            bounds_min: [0.0; 3],
            bounds_max: [39.37; 3],
            solid_tris: vec![MeshTri {
                material: "terrain_opaque".into(),
                pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                norm: [[0.0, 0.0, 1.0]; 3],
                uv: [[0.0, 0.0]; 3],
            }],
            translucent_tris: Vec::new(),
            has_cutout: false,
        };
        let art_a = emit_chunk_smds_and_qcs(&cm, &[], &work_a.join("smd"), &cat_a, 64, "gm_map_alpha").unwrap();
        let art_b = emit_chunk_smds_and_qcs(&cm, &[], &work_b.join("smd"), &cat_b, 64, "gm_map_beta").unwrap();
        let qc_a = std::fs::read_to_string(&art_a.qc_path).unwrap();
        let qc_b = std::fs::read_to_string(&art_b.qc_path).unwrap();
        assert!(qc_a.contains("$cdmaterials \"models/custom_mc/gm_map_alpha\""));
        assert!(qc_b.contains("$cdmaterials \"models/custom_mc/gm_map_beta\""));

        let vmf_a = work_a.join("gm_map_alpha.vmf");
        build_vmf(&VmfBuildParams {
            output_vmf: &vmf_a,
            category: &cat_a,
            bounds_min: [-100.0, -100.0, 0.0],
            bounds_max: [100.0, 100.0, 200.0],
            chunks: &[art_a],
            all_solid_boxes: &[],
            ladder_boxes: &[],
            water_pools: &[[0.0, 0.0, 0.0, 39.37, 39.37, 35.0]],
            hazard_boxes: &[],
            core_occluders: &[],
            doors: &[],
            buttons: &[],
            lights: &[],
            spawn_points_source: &[[0.0, 0.0, 48.0]],
            lighting_style: "minecraft",
            hq_light: false,
            scale_factor: 39.37,
            force_vmf_brushes: false,
        })
        .unwrap();
        let vmf_a_txt = std::fs::read_to_string(&vmf_a).unwrap();
        assert!(vmf_a_txt.contains("models/custom_mc/gm_map_alpha/gm_map_alpha_chunk_0_0.mdl"));
        assert!(vmf_a_txt.contains("custom_mc/gm_map_alpha/mc_water"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_work_dir_outside_addon_and_gma_excludes_work_files() {
        use crate::compiler::{create_gma_addon, ToolPaths};

        // 1. --name castle --output-dir C:\exports\castle must place _work outside addon_dir
        let (addon_same, work_same) =
            crate::resolve_addon_and_work_dirs(std::path::Path::new(r"C:\exports\castle"), "castle");
        assert_eq!(addon_same, std::path::PathBuf::from(r"C:\exports\castle"));
        assert_eq!(work_same, std::path::PathBuf::from(r"C:\exports\_work\castle"));
        assert!(!work_same.starts_with(&addon_same));

        let (addon_sub, work_sub) =
            crate::resolve_addon_and_work_dirs(std::path::Path::new(r"C:\exports"), "castle");
        assert_eq!(addon_sub, std::path::PathBuf::from(r"C:\exports\castle"));
        assert_eq!(work_sub, std::path::PathBuf::from(r"C:\exports\_work\castle"));
        assert!(!work_sub.starts_with(&addon_sub));

        // 2. Both native GMA packer and gmad.exe metadata exclude working files
        let tmp = make_temp_test_dir("gma_exclude_work");
        let addon_dir = tmp.join("castle");
        std::fs::create_dir_all(addon_dir.join("maps/thumb")).unwrap();
        std::fs::create_dir_all(addon_dir.join("materials/models/custom_mc/gm_castle")).unwrap();
        std::fs::create_dir_all(addon_dir.join("models/custom_mc/gm_castle")).unwrap();
        std::fs::create_dir_all(addon_dir.join("lua/autorun")).unwrap();
        std::fs::create_dir_all(addon_dir.join("_work/castle/smd")).unwrap();

        // Real game resources
        std::fs::write(addon_dir.join("maps/gm_castle.bsp"), b"VBSP").unwrap();
        std::fs::write(addon_dir.join("maps/thumb/gm_castle.png"), b"PNG").unwrap();
        std::fs::write(
            addon_dir.join("materials/models/custom_mc/gm_castle/terrain_opaque.vmt"),
            b"VMT",
        )
        .unwrap();
        std::fs::write(
            addon_dir.join("models/custom_mc/gm_castle/chunk_0_0.mdl"),
            b"IDST",
        )
        .unwrap();
        std::fs::write(addon_dir.join("lua/autorun/mcgm_physics.lua"), b"-- lua").unwrap();

        // Stray work / intermediate files that must NOT enter GMA
        std::fs::write(addon_dir.join("_work/castle/smd/chunk_0_0.smd"), b"SMD").unwrap();
        std::fs::write(addon_dir.join("_work/castle/smd/chunk_0_0.qc"), b"QC").unwrap();
        std::fs::write(addon_dir.join("_work/castle/gm_castle.bsp"), b"INTERMEDIATE_BSP").unwrap();
        std::fs::write(addon_dir.join("_work/castle/mcgm_debug.log"), b"LOG").unwrap();
        std::fs::write(addon_dir.join("maps/gm_castle.vmf"), b"VMF").unwrap();
        std::fs::write(addon_dir.join("maps/compile.log"), b"LOG2").unwrap();
        std::fs::write(addon_dir.join("models/stray.smd"), b"SMD2").unwrap();
        std::fs::write(addon_dir.join("models/stray.qc"), b"QC2").unwrap();

        write_addon_metadata(&addon_dir, "gm_castle").unwrap();
        let meta_txt = std::fs::read_to_string(addon_dir.join("addon.json")).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta_txt).unwrap();
        let ignores: Vec<&str> = meta["ignore"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for pat in ["_work/*", "*.vmf", "*.smd", "*.qc", "*.log"] {
            assert!(ignores.contains(&pat), "addon.json ignore missing {}", pat);
        }

        // Native packer
        let native_gma = tmp.join("castle_native.gma");
        pack_gma_v3_native(&addon_dir, &native_gma, "MCGM - gm_castle", "{}").unwrap();
        let bytes = std::fs::read(&native_gma).unwrap();
        let text_view = String::from_utf8_lossy(&bytes);
        assert!(text_view.contains("maps/gm_castle.bsp"));
        assert!(text_view.contains("models/custom_mc/gm_castle/chunk_0_0.mdl"));
        assert!(text_view.contains("materials/models/custom_mc/gm_castle/terrain_opaque.vmt"));
        assert!(text_view.contains("lua/autorun/mcgm_physics.lua"));
        assert!(!text_view.contains("_work/"));
        assert!(!text_view.contains("INTERMEDIATE_BSP"));
        assert!(!text_view.contains(".smd"));
        assert!(!text_view.contains(".qc"));
        assert!(!text_view.contains(".log"));
        assert!(!text_view.contains(".vmf"));

        // gmad.exe path (simulated via mock gmad.cmd verifying addon.json ignore rules exist)
        let mock_gmad = tmp.join("mock_gmad.cmd");
        std::fs::write(
            &mock_gmad,
            "@echo off\r\n\
            if not exist \"%~3\\addon.json\" exit /b 2\r\n\
            findstr /C:\"_work/*\" \"%~3\\addon.json\" >nul || exit /b 3\r\n\
            echo GMAD_OK>\"%~5\"\r\n",
        )
        .unwrap();
        let tools_gmad = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: std::path::PathBuf::new(),
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: Some(mock_gmad),
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };
        let gmad_out = tmp.join("castle_gmad.gma");
        create_gma_addon(&tools_gmad, &addon_dir, &gmad_out, "gm_castle", |_| {}).unwrap();
        assert_eq!(std::fs::read_to_string(&gmad_out).unwrap().trim(), "GMAD_OK");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_vvis_and_vrad_failures_propagate_and_preserve_published_bsp() {
        use crate::compiler::{compile_bsp_map_cancellable, ToolPaths};

        let tmp = make_temp_test_dir("vvis_vrad_errors");
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        let game_dir = tmp.join("game");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(addon_dir.join("maps")).unwrap();
        std::fs::create_dir_all(game_dir.join("maps")).unwrap();

        let vmf_path = work_dir.join("gm_test.vmf");
        std::fs::write(&vmf_path, "worldspawn {}").unwrap();

        let published_addon_bsp = addon_dir.join("maps/gm_test.bsp");
        let published_game_bsp = game_dir.join("maps/gm_test.bsp");
        let old_bsp = b"PUBLISHED_OLD_BSP_DATA";
        std::fs::write(&published_addon_bsp, old_bsp).unwrap();
        std::fs::write(&published_game_bsp, old_bsp).unwrap();

        let vbsp_cmd = tmp.join("vbsp++.cmd");
        std::fs::write(
            &vbsp_cmd,
            "@echo off\r\necho NEW_TEMP_BSP>gm_test.bsp\r\necho PRT>gm_test.prt\r\nexit /b 0\r\n",
        )
        .unwrap();

        let vvis_fail = tmp.join("vvis_fail.cmd");
        std::fs::write(&vvis_fail, "@echo off\r\necho vvis error 1>&2\r\nexit /b 7\r\n").unwrap();

        let vvis_ok = tmp.join("vvis++.cmd");
        std::fs::write(&vvis_ok, "@echo off\r\nexit /b 0\r\n").unwrap();

        let vrad_cpu_retry_ok = tmp.join("vrad++_retry.cmd");
        std::fs::write(
            &vrad_cpu_retry_ok,
            "@echo off\r\n\
            echo %* | findstr /C:\"-cpu\" >nul\r\n\
            if errorlevel 1 ( echo gpu crash 1>&2 & exit /b 3 )\r\n\
            exit /b 0\r\n",
        )
        .unwrap();

        let vrad_always_fail = tmp.join("vrad++_fail.cmd");
        std::fs::write(
            &vrad_always_fail,
            "@echo off\r\necho vrad fatal 1>&2\r\nexit /b 42\r\n",
        )
        .unwrap();

        // 1. VVIS fails with code 7 -> returns Err with stage & code, preserves old BSP
        let tools_vvis_fail = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: std::path::PathBuf::new(),
            vbsp: vbsp_cmd.clone(),
            vvis: vvis_fail,
            vrad: vrad_cpu_retry_ok.clone(),
            gmad: None,
            game_dir: game_dir.clone(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };
        let vvis_err = compile_bsp_map_cancellable(
            &tools_vvis_fail,
            &vmf_path,
            "gm_test",
            &addon_dir,
            true,
            false,
            false,
            "minecraft",
            None,
            |_| {},
        )
        .unwrap_err();
        assert!(vvis_err.contains("VVIS") && vvis_err.contains("7"), "got: {}", vvis_err);
        assert_eq!(std::fs::read(&published_addon_bsp).unwrap(), old_bsp);
        assert_eq!(std::fs::read(&published_game_bsp).unwrap(), old_bsp);

        // 2. VRAD++ GPU pass fails -> CPU fallback succeeds -> published BSP updated
        let tools_vrad_retry = ToolPaths {
            vvis: vvis_ok.clone(),
            vrad: vrad_cpu_retry_ok,
            ..tools_vvis_fail.clone()
        };
        let mut retry_logs = Vec::new();
        let ok_bsp = compile_bsp_map_cancellable(
            &tools_vrad_retry,
            &vmf_path,
            "gm_test",
            &addon_dir,
            true,
            false,
            false,
            "minecraft",
            None,
            |line| retry_logs.push(line.to_string()),
        )
        .unwrap();
        assert_eq!(ok_bsp, published_game_bsp);
        assert!(retry_logs.iter().any(|l| l.contains("retrying with -cpu fallback")));
        assert!(std::fs::read_to_string(&published_addon_bsp).unwrap().contains("NEW_TEMP_BSP"));
        assert!(std::fs::read_to_string(&published_game_bsp).unwrap().contains("NEW_TEMP_BSP"));

        // 3. VRAD++ GPU + CPU fallback both fail with code 42 -> Err with stage & code, preserves old BSP
        std::fs::write(&published_addon_bsp, old_bsp).unwrap();
        std::fs::write(&published_game_bsp, old_bsp).unwrap();
        let tools_vrad_fail = ToolPaths {
            vvis: vvis_ok,
            vrad: vrad_always_fail,
            ..tools_vvis_fail
        };
        let vrad_err = compile_bsp_map_cancellable(
            &tools_vrad_fail,
            &vmf_path,
            "gm_test",
            &addon_dir,
            true,
            false,
            false,
            "minecraft",
            None,
            |_| {},
        )
        .unwrap_err();
        assert!(
            vrad_err.contains("VRAD++") && vrad_err.contains("42"),
            "got: {}",
            vrad_err
        );
        assert_eq!(std::fs::read(&published_addon_bsp).unwrap(), old_bsp);
        assert_eq!(std::fs::read(&published_game_bsp).unwrap(), old_bsp);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_gui_launch_coords_error_disables_autostart_and_preserves_missing_coords() {
        use clap::{CommandFactory, FromArgMatches};

        // 1. Typo in coordinate number
        let m_typo = CliArgs::command()
            .try_get_matches_from(["mcgm", "--gui", "--auto-start", "--coords", "0", "64", "0", "10", "70", "abc"])
            .unwrap();
        let a_typo = CliArgs::from_arg_matches(&m_typo).unwrap();
        let cfg_typo = crate::build_gui_launch_config(&a_typo, &m_typo);
        assert!(!cfg_typo.auto_start, "auto_start must stop when --coords has a typo");
        assert!(cfg_typo.coords.is_none());
        assert!(cfg_typo.startup_error.as_deref().unwrap_or("").contains("abc"));

        // 2. Incomplete coordinate count
        let m_inc = CliArgs::command()
            .try_get_matches_from(["mcgm", "--gui", "--auto-start", "--coords", "0", "64", "0", "10"])
            .unwrap();
        let a_inc = CliArgs::from_arg_matches(&m_inc).unwrap();
        let cfg_inc = crate::build_gui_launch_config(&a_inc, &m_inc);
        assert!(!cfg_inc.auto_start, "auto_start must stop when --coords is incomplete");
        assert!(cfg_inc.coords.is_none());
        assert!(cfg_inc.startup_error.as_deref().unwrap_or("").contains("got 4"));

        // 3. Omitted --coords preserves normal selection / auto_start
        let m_none = CliArgs::command()
            .try_get_matches_from(["mcgm", "--gui", "--auto-start"])
            .unwrap();
        let a_none = CliArgs::from_arg_matches(&m_none).unwrap();
        let cfg_none = crate::build_gui_launch_config(&a_none, &m_none);
        assert!(cfg_none.auto_start);
        assert!(cfg_none.coords.is_none());
        assert!(cfg_none.startup_error.is_none());
    }

    #[test]
    fn test_gui_launch_explicit_cli_overrides_pipeline_options() {
        use crate::gui::{resolve_gui_pipeline_options, PersistedGuiState};
        use clap::{CommandFactory, FromArgMatches};

        let matches = CliArgs::command()
            .try_get_matches_from([
                "mcgm",
                "--gui",
                "--auto-start",
                "--chunk-size",
                "16",
                "--max-workers",
                "1",
                "--max-convex",
                "48",
                "--reuse-cache",
            ])
            .unwrap();
        let args = CliArgs::from_arg_matches(&matches).unwrap();
        let cfg = crate::build_gui_launch_config(&args, &matches);
        assert_eq!(cfg.chunk_size, Some(16));
        assert_eq!(cfg.max_workers, Some(1));
        assert_eq!(cfg.max_convex, Some(48));
        assert_eq!(cfg.reuse_cache, Some(true));
        assert!(cfg.auto_start);

        let saved = PersistedGuiState {
            build_preset: Some("balanced".to_string()),
            name: Some("saved_name".to_string()),
            ..Default::default()
        };
        let opts = resolve_gui_pipeline_options(Some(&saved), &cfg);
        assert_eq!(opts.chunk_size, 16);
        assert_eq!(opts.max_workers, 1);
        assert_eq!(opts.max_convex, 48);
        assert!(opts.reuse_cache);
    }

    #[test]
    fn test_max_convex_pieces_config_priority_and_qc_splitting() {
        use crate::compiler::{load_tool_paths, DEFAULT_MAX_CONVEX_PIECES};

        let tmp = make_temp_test_dir("max_convex_cfg");
        let cfg_4 = tmp.join("config_4.json");
        let cfg_64 = tmp.join("config_64.json");
        let cfg_empty = tmp.join("config_empty.json");
        std::fs::write(&cfg_4, r#"{"max_convex_pieces": 4}"#).unwrap();
        std::fs::write(&cfg_64, r#"{"max_convex_pieces": 64}"#).unwrap();
        std::fs::write(&cfg_empty, r#"{}"#).unwrap();

        let t4 = load_tool_paths(Some(&cfg_4));
        let t64 = load_tool_paths(Some(&cfg_64));
        let tdef = load_tool_paths(Some(&cfg_empty));
        assert_eq!(t4.max_convex_pieces, 4);
        assert_eq!(t64.max_convex_pieces, 64);
        assert_eq!(tdef.max_convex_pieces, DEFAULT_MAX_CONVEX_PIECES);

        // Priority: explicit CLI > config.json > default (2048)
        assert_eq!(crate::resolve_effective_max_convex(12, t4.max_convex_pieces), 12);
        assert_eq!(crate::resolve_effective_max_convex(0, t4.max_convex_pieces), 4);
        assert_eq!(crate::resolve_effective_max_convex(0, t64.max_convex_pieces), 64);
        assert_eq!(crate::resolve_effective_max_convex(0, 0), 2048);

        let cm = crate::mesher::ChunkMesh {
            cx: 0,
            cz: 0,
            name: "chunk_0_0".to_string(),
            origin_source: [0.0; 3],
            bounds_min: [0.0; 3],
            bounds_max: [275.59, 39.37, 39.37],
            solid_tris: vec![MeshTri {
                material: "terrain_opaque".into(),
                pos: [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                norm: [[0.0, 0.0, 1.0]; 3],
                uv: [[0.0, 0.0]; 3],
            }],
            translucent_tris: Vec::new(),
            has_cutout: false,
        };
        let boxes: Vec<CollisionBox> = (0..7)
            .map(|i| CollisionBox {
                box_coords: [i as f64 * 39.37, 0.0, 0.0, (i + 1) as f64 * 39.37, 39.37, 39.37],
                block_class: "full".into(),
                is_walkable: true,
                is_opaque_roof: true,
                surface_prop: SurfaceProp::Rock,
            })
            .collect();
        let eff_max = crate::resolve_effective_max_convex(0, t4.max_convex_pieces);
        let art = emit_chunk_smds_and_qcs(&cm, &boxes, &tmp.join("smd"), "custom_mc", eff_max, "convex_map").unwrap();
        let main_qc = std::fs::read_to_string(&art.qc_path).unwrap();
        assert!(main_qc.contains("$maxconvexpieces 4"), "QC must use config max_convex_pieces=4, got:\n{}", main_qc);
        assert_eq!(art.extra_collision_models.len(), 1, "7 boxes at max=4 must split into 4+3");
        let extra_qc = std::fs::read_to_string(&art.extra_collision_models[0].0).unwrap();
        assert!(extra_qc.contains("$maxconvexpieces 4"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reuse_cache_invalidates_atlas_and_vtf_when_jar_png_changes() {
        use crate::atlas::compute_atlas_cache_hash_with_assets;
        use crate::compiler::{compile_materials, ToolPaths};
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let tmp = make_temp_test_dir("reuse_cache_jar_png");
        let jar_path = tmp.join("client.jar");
        let work_dir = tmp.join("work");
        let addon_dir = tmp.join("addon");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&addon_dir).unwrap();

        let write_jar = |color: [u8; 4]| {
            let f = std::fs::File::create(&jar_path).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            zw.start_file("assets/minecraft/textures/block/stone.png", opts).unwrap();
            let img = ImageBuffer::from_pixel(16, 16, Rgba(color));
            zw.write_all(&encode_png_bytes(&img)).unwrap();
            zw.start_file("version.json", opts).unwrap();
            zw.write_all(br#"{"world_version": 3465}"#).unwrap();
            zw.finish().unwrap();
        };

        let calls_log = tmp.join("vtf_calls.txt");
        let mock_vtfcmd = tmp.join("mock_vtfcmd.cmd");
        std::fs::write(
            &mock_vtfcmd,
            format!(
                "@echo off\r\necho %~n2>>\"{}\"\r\ncopy /y \"%~2\" \"%~4\\%~n2.vtf\" >nul\r\n",
                calls_log.display()
            ),
        )
        .unwrap();

        let tools = ToolPaths {
            studiomdl: std::path::PathBuf::new(),
            vtfcmd: mock_vtfcmd,
            vbsp: std::path::PathBuf::new(),
            vvis: std::path::PathBuf::new(),
            vrad: std::path::PathBuf::new(),
            gmad: None,
            game_dir: std::path::PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: 2048,
        };

        let mut reqs = HashSet::new();
        reqs.insert(TexRequestKey {
            tex_ref: "minecraft:block/stone".to_string(),
            tint_rgb: None,
        });

        // 1. Initial JAR with red stone.png
        write_jar([230, 10, 10, 255]);
        let assets1 = AssetIndex::discover_and_load(Some(&jar_path), None, &[]);
        let h1 = compute_atlas_cache_hash_with_assets(&reqs, &assets1, 39.37, "minecraft", 8);
        let atlas1 = build_texture_atlas(&reqs, &assets1, 8);
        let atlas_png_1 = work_dir.join(format!("atlas_{}.png", h1));
        let sign_png = work_dir.join("sign_atlas.png");
        crate::atlas::save_png_fast(&atlas1.image, &atlas_png_1).unwrap();
        crate::atlas::save_png_fast(&ImageBuffer::new(16, 16), &sign_png).unwrap();
        compile_materials(&tools, "custom_mc", &atlas_png_1, &sign_png, &HashMap::new(), &[], &addon_dir, false, true).unwrap();

        // 2. Identical JAR -> same hash & VTF cache hit (VTFCmd skipped for atlas)
        let _ = std::fs::remove_file(&calls_log);
        let assets1_rep = AssetIndex::discover_and_load(Some(&jar_path), None, &[]);
        let h1_rep = compute_atlas_cache_hash_with_assets(&reqs, &assets1_rep, 39.37, "minecraft", 8);
        assert_eq!(h1_rep, h1);
        compile_materials(&tools, "custom_mc", &atlas_png_1, &sign_png, &HashMap::new(), &[], &addon_dir, false, true).unwrap();
        let rep_log = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert!(!rep_log.contains(&format!("atlas_{}", h1)), "identical assets must reuse cached VTF");

        // 3. Replace stone.png bytes in JAR under same name with blue pixels -> new hash, new atlas, new VTF
        let _ = std::fs::remove_file(&calls_log);
        write_jar([10, 30, 240, 255]);
        let assets2 = AssetIndex::discover_and_load(Some(&jar_path), None, &[]);
        let h2 = compute_atlas_cache_hash_with_assets(&reqs, &assets2, 39.37, "minecraft", 8);
        assert_ne!(h2, h1, "changing PNG bytes in JAR must invalidate atlas cache hash");
        let atlas2 = build_texture_atlas(&reqs, &assets2, 8);
        let has_blue = atlas2.image.pixels().any(|p| p[2] > 200 && p[0] < 50);
        assert!(has_blue, "new atlas must contain updated blue pixels");
        let atlas_png_2 = work_dir.join(format!("atlas_{}.png", h2));
        crate::atlas::save_png_fast(&atlas2.image, &atlas_png_2).unwrap();
        compile_materials(&tools, "custom_mc", &atlas_png_2, &sign_png, &HashMap::new(), &[], &addon_dir, false, true).unwrap();
        let log2 = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert!(log2.contains(&format!("atlas_{}", h2)), "changed PNG must invoke VTFCmd for new atlas");

        // 4. Even if PNG path has the same filename, changing its bytes invalidates .vtf.src_hash
        let _ = std::fs::remove_file(&calls_log);
        crate::atlas::save_png_fast(&atlas2.image, &atlas_png_1).unwrap();
        compile_materials(&tools, "custom_mc", &atlas_png_1, &sign_png, &HashMap::new(), &[], &addon_dir, false, true).unwrap();
        let log3 = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert!(log3.contains(&format!("atlas_{}", h1)), "vtf.src_hash mismatch must re-run VTFCmd");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_custom_dimension_passthrough_gui_and_anvil_region_dir() {
        use crate::anvil::resolve_dimension_region_dir;
        use crate::gui::resolve_gui_pipeline_options;
        use clap::{CommandFactory, FromArgMatches};

        let tmp = make_temp_test_dir("custom_dim_passthrough");
        let ov_reg = tmp.join("region");
        let neth_reg = tmp.join("DIM-1/region");
        let end_reg = tmp.join("DIM1/region");
        let moon_reg = tmp.join("dimensions/mymod/moon/region");
        let nested_reg = tmp.join("dimensions/mymod/outer/rim/region");
        std::fs::create_dir_all(&ov_reg).unwrap();
        std::fs::create_dir_all(&neth_reg).unwrap();
        std::fs::create_dir_all(&end_reg).unwrap();
        std::fs::create_dir_all(&moon_reg).unwrap();
        std::fs::create_dir_all(&nested_reg).unwrap();

        assert_eq!(resolve_dimension_region_dir(&tmp, "overworld").unwrap(), ov_reg);
        assert_eq!(resolve_dimension_region_dir(&tmp, "nether").unwrap(), neth_reg);
        assert_eq!(resolve_dimension_region_dir(&tmp, "end").unwrap(), end_reg);
        assert_eq!(resolve_dimension_region_dir(&tmp, "mymod:moon").unwrap(), moon_reg);
        assert_eq!(resolve_dimension_region_dir(&tmp, "mymod:outer/rim").unwrap(), nested_reg);

        let matches = CliArgs::command()
            .try_get_matches_from(["mcgm", "--gui", "--dimension", "mymod:moon"])
            .unwrap();
        let args = CliArgs::from_arg_matches(&matches).unwrap();
        let cfg = crate::build_gui_launch_config(&args, &matches);
        assert_eq!(cfg.dimension.as_deref(), Some("mymod:moon"));
        let opts = resolve_gui_pipeline_options(None, &cfg);
        assert_eq!(opts.dimension, "mymod:moon");
        assert_eq!(resolve_dimension_region_dir(&tmp, &opts.dimension).unwrap(), moon_reg);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_staged_model_guard_preserves_backup_and_restores_others_on_partial_failure() {
        use crate::compiler::StagedModelGuard;

        let tmp = make_temp_test_dir("staged_guard_partial_fail");
        let game_dir = tmp.join("game");
        let backup_root = tmp.join("backups");
        let model_dir = game_dir.join("models/custom_mc/gm_map");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::create_dir_all(&backup_root).unwrap();

        let mdl_path = model_dir.join("chunk_0_0.mdl");
        let vvd_path = model_dir.join("chunk_0_0.vvd");
        let phy_path = model_dir.join("chunk_0_0.phy");
        let new_tmp_mdl = model_dir.join("chunk_1_0.mdl");

        let orig_mdl = b"ORIGINAL_MDL_CONTENTS_KEEP_BACKUP";
        let orig_vvd = b"ORIGINAL_VVD_CONTENTS";
        let orig_phy = b"ORIGINAL_PHY_CONTENTS";
        std::fs::write(&mdl_path, orig_mdl).unwrap();
        std::fs::write(&vvd_path, orig_vvd).unwrap();
        std::fs::write(&phy_path, orig_phy).unwrap();

        let rels = vec![
            "models/custom_mc/gm_map/chunk_0_0.mdl".to_string(),
            "models/custom_mc/gm_map/chunk_1_0.mdl".to_string(),
        ];

        // 1. Explicit finish() path: place a directory where chunk_0_0.mdl should be restored
        let guard = StagedModelGuard::prepare(&game_dir, &rels, &backup_root).unwrap();
        std::fs::write(&vvd_path, b"CORRUPTED_VVD").unwrap();
        std::fs::write(&phy_path, b"CORRUPTED_PHY").unwrap();
        std::fs::write(&new_tmp_mdl, b"TEMP_MODEL").unwrap();
        std::fs::remove_file(&mdl_path).unwrap();
        std::fs::create_dir_all(&mdl_path).unwrap();

        let err = guard
            .finish()
            .expect_err("restoring into a directory path must return Err");
        assert!(
            err.contains("chunk_0_0.mdl"),
            "error must name failed target path: {}",
            err
        );
        assert_eq!(std::fs::read(&vvd_path).unwrap(), orig_vvd);
        assert_eq!(std::fs::read(&phy_path).unwrap(), orig_phy);
        assert!(!new_tmp_mdl.exists(), "temporary file must be removed even on partial failure");

        let preserved_baks: Vec<std::path::PathBuf> = std::fs::read_dir(&backup_root)
            .unwrap()
            .flatten()
            .flat_map(|d| std::fs::read_dir(d.path()).ok().into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bak"))
            .collect();
        assert_eq!(
            preserved_baks.len(),
            1,
            "only the un-restorable backup should remain, found: {:?}",
            preserved_baks
        );
        let preserved_path_str = preserved_baks[0].to_string_lossy().into_owned();
        assert!(
            err.contains(&preserved_path_str),
            "error must expose preserved backup path '{}': {}",
            preserved_path_str,
            err
        );
        assert_eq!(
            std::fs::read(&preserved_baks[0]).unwrap(),
            orig_mdl,
            "preserved backup must contain exact original bytes"
        );

        // Clean up collision and verify Drop fallback also preserves backups on partial failure
        std::fs::remove_dir_all(&mdl_path).unwrap();
        std::fs::write(&mdl_path, orig_mdl).unwrap();
        let drop_backup_root = tmp.join("drop_backups");
        std::fs::create_dir_all(&drop_backup_root).unwrap();
        {
            let _drop_guard = StagedModelGuard::prepare(&game_dir, &rels, &drop_backup_root).unwrap();
            std::fs::write(&vvd_path, b"DROP_CORRUPT_VVD").unwrap();
            std::fs::remove_file(&mdl_path).unwrap();
            std::fs::create_dir_all(&mdl_path).unwrap();
        }
        assert_eq!(std::fs::read(&vvd_path).unwrap(), orig_vvd);
        let drop_baks: Vec<std::path::PathBuf> = std::fs::read_dir(&drop_backup_root)
            .unwrap()
            .flatten()
            .flat_map(|d| std::fs::read_dir(d.path()).ok().into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bak"))
            .collect();
        assert_eq!(drop_baks.len(), 1);
        assert_eq!(std::fs::read(&drop_baks[0]).unwrap(), orig_mdl);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_partial_success_bsp_ready_gma_failed_contract_cli_log_and_gui() {
        let ok_summary = crate::PipelineSummary {
            bsp_path: std::path::PathBuf::from(r"C:\exports\gm_demo.bsp"),
            gma_path: Some(std::path::PathBuf::from(r"C:\exports\gm_demo.gma")),
            gma_error: None,
            chunk_count: 5,
            door_count: 2,
            total_secs: 3.25,
            stage_timings: vec![],
        };
        assert_eq!(ok_summary.outcome_status(), crate::ExportOutcomeStatus::Success);
        assert_eq!(ok_summary.outcome_status().exit_code(), crate::EXIT_CODE_SUCCESS);
        assert_eq!(ok_summary.status_log_line(), "Status: SUCCESS");

        let partial = crate::PipelineSummary {
            bsp_path: std::path::PathBuf::from(r"C:\exports\gm_demo.bsp"),
            gma_path: None,
            gma_error: Some("disk full".to_string()),
            chunk_count: 5,
            door_count: 2,
            total_secs: 3.25,
            stage_timings: vec![],
        };
        assert_eq!(partial.outcome_status(), crate::ExportOutcomeStatus::PartialSuccess);
        assert_eq!(
            partial.outcome_status().exit_code(),
            crate::EXIT_CODE_PARTIAL_SUCCESS
        );
        let status_line = partial.status_log_line();
        assert!(status_line.starts_with("Status: PARTIAL_SUCCESS"));
        assert!(!status_line.contains("Status: SUCCESS"));

        let cli_report = partial.format_cli_report();
        assert!(cli_report.contains("PARTIAL_SUCCESS"));
        assert!(cli_report.contains("gm_demo.bsp"));
        assert!(cli_report.contains("disk full"));

        let gui_ru = format_done_summary(UiLang::Ru, &partial);
        let gui_en = format_done_summary(UiLang::En, &partial);
        assert!(gui_ru.contains("Частичный успех") && gui_ru.contains("gm_demo.bsp") && gui_ru.contains("disk full"));
        assert!(gui_en.contains("Partial success") && gui_en.contains("gm_demo.bsp") && gui_en.contains("disk full"));
        assert!(!tr_text(UiLang::Ru, "status_partial").is_empty());
        assert!(!tr_text(UiLang::En, "partial_title").is_empty());
    }
}
