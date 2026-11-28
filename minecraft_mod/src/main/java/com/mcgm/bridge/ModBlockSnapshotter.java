package com.mcgm.bridge;

import com.mojang.blaze3d.systems.RenderSystem;
import net.fabricmc.fabric.api.renderer.v1.Renderer;
import net.fabricmc.fabric.api.renderer.v1.RendererAccess;
import net.fabricmc.fabric.api.renderer.v1.material.BlendMode;
import net.fabricmc.fabric.api.renderer.v1.material.RenderMaterial;
import net.fabricmc.fabric.api.renderer.v1.mesh.Mesh;
import net.fabricmc.fabric.api.renderer.v1.mesh.MeshBuilder;
import net.fabricmc.fabric.api.renderer.v1.mesh.MutableQuadView;
import net.fabricmc.fabric.api.renderer.v1.mesh.QuadEmitter;
import net.fabricmc.fabric.api.renderer.v1.mesh.QuadView;
import net.fabricmc.fabric.api.renderer.v1.model.FabricBakedModel;
import net.fabricmc.fabric.api.renderer.v1.model.SpriteFinder;
import net.fabricmc.fabric.api.renderer.v1.render.RenderContext;
import net.minecraft.block.BlockState;
import net.minecraft.block.entity.BlockEntity;
import net.minecraft.block.entity.BlockEntityType;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.color.block.BlockColors;
import net.minecraft.client.render.LightmapTextureManager;
import net.minecraft.client.render.OverlayTexture;
import net.minecraft.client.render.RenderLayer;
import net.minecraft.client.render.RenderLayers;
import net.minecraft.client.render.VertexConsumer;
import net.minecraft.client.render.VertexConsumerProvider;
import net.minecraft.client.render.VertexFormat;
import net.minecraft.client.render.block.entity.BlockEntityRenderDispatcher;
import net.minecraft.client.render.block.entity.BlockEntityRenderer;
import net.minecraft.client.render.model.BakedModel;
import net.minecraft.client.render.model.BakedQuad;
import net.minecraft.client.texture.NativeImage;
import net.minecraft.client.texture.Sprite;
import net.minecraft.client.texture.SpriteAtlasTexture;
import net.minecraft.client.texture.SpriteContents;
import net.minecraft.client.util.math.MatrixStack;
import net.minecraft.registry.Registries;
import net.minecraft.resource.Resource;
import net.minecraft.resource.ResourceManager;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Direction;
import net.minecraft.util.math.random.Random;
import net.minecraft.util.shape.VoxelShape;
import net.minecraft.world.BlockRenderView;
import net.minecraft.world.World;
import org.joml.Matrix3f;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector4f;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Instant;
import java.time.format.DateTimeFormatter;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Deque;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.function.Consumer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Universal v1 static capture of modded blocks from a running Fabric 1.20.1 client.
 * Captures baked models (Vanilla + Fabric Renderer API), BlockEntityRenderer output,
 * active resource-pack/runtime textures, UVs, BlockColors tints, render layers,
 * and VoxelShape collision bounds into a portable, versioned snapshot package.
 */
public final class ModBlockSnapshotter {
    public static final int FORMAT_VERSION = 1;
    public static final String COORD_SYSTEM = "mc_world_y_up_rhs";

    private static final Pattern LAYER_TEX_PATTERN = Pattern.compile("texture\\[([^\\s\\],]+)");

    private ModBlockSnapshotter() {
    }

    public enum CaptureStatus {
        FULL("full"),
        PARTIAL("partial"),
        FALLBACK("fallback");

        public final String wireName;

        CaptureStatus(String wireName) {
            this.wireName = wireName;
        }
    }

    public static final class CapturedTri {
        public final float[][] pos; // [3][3] in block-local MC coords
        public final float[] normal; // [3]
        public final float[][] uv; // [3][2] normalized [0..1], v down
        public final String textureId;
        public final String renderLayer; // "solid", "cutout", "translucent"
        public final int[] tintRgb; // nullable [r, g, b]

        public CapturedTri(
            float[][] pos,
            float[] normal,
            float[][] uv,
            String textureId,
            String renderLayer,
            int[] tintRgb
        ) {
            this.pos = new float[3][3];
            this.uv = new float[3][2];
            for (int i = 0; i < 3; i++) {
                System.arraycopy(pos[i], 0, this.pos[i], 0, 3);
                System.arraycopy(uv[i], 0, this.uv[i], 0, 2);
            }
            this.normal = Arrays.copyOf(normal, 3);
            this.textureId = (textureId != null && !textureId.isEmpty()) ? textureId : "missing";
            this.renderLayer = normalizeLayerName(renderLayer);
            if (tintRgb != null && tintRgb.length == 3
                && (tintRgb[0] != 255 || tintRgb[1] != 255 || tintRgb[2] != 255)) {
                this.tintRgb = new int[]{
                    clamp255(tintRgb[0]),
                    clamp255(tintRgb[1]),
                    clamp255(tintRgb[2])
                };
            } else {
                this.tintRgb = null;
            }
        }
    }

    public static final class TextureEntry {
        public final String id;
        public final String relPath;
        public final String sourceId;
        public final int width;
        public final int height;
        public final byte[] pngBytes;

        TextureEntry(String id, String relPath, String sourceId, int width, int height, byte[] pngBytes) {
            this.id = id;
            this.relPath = relPath;
            this.sourceId = sourceId;
            this.width = width;
            this.height = height;
            this.pngBytes = pngBytes;
        }
    }

    public static final class MeshEntry {
        public final String id;
        public final String relPath;
        public final List<CapturedTri> triangles;

        MeshEntry(String id, String relPath, List<CapturedTri> triangles) {
            this.id = id;
            this.relPath = relPath;
            this.triangles = triangles;
        }
    }

    public static final class BlockInstanceEntry {
        public final int x;
        public final int y;
        public final int z;
        public final String blockId;
        public final boolean hasBlockEntity;
        public final CaptureStatus status;
        public final String captureReason;
        public final String meshId;
        public final boolean hasCollision;
        public final List<double[]> collisionBoxes;

        BlockInstanceEntry(
            int x, int y, int z,
            String blockId,
            boolean hasBlockEntity,
            CaptureStatus status,
            String captureReason,
            String meshId,
            List<double[]> collisionBoxes
        ) {
            this.x = x;
            this.y = y;
            this.z = z;
            this.blockId = blockId;
            this.hasBlockEntity = hasBlockEntity;
            this.status = status;
            this.captureReason = captureReason;
            this.meshId = meshId;
            this.collisionBoxes = collisionBoxes;
            this.hasCollision = !collisionBoxes.isEmpty();
        }
    }

    public static final class SnapshotResult {
        public final Path snapshotDir;
        public final Path manifestPath;
        public final int capturedBlocks;
        public final int fullBlocks;
        public final int partialBlocks;
        public final int fallbackBlocks;
        public final int uniqueMeshes;
        public final int uniqueTextures;
        public final List<String> issueSamples;

        SnapshotResult(
            Path snapshotDir,
            Path manifestPath,
            int capturedBlocks,
            int fullBlocks,
            int partialBlocks,
            int fallbackBlocks,
            int uniqueMeshes,
            int uniqueTextures,
            List<String> issueSamples
        ) {
            this.snapshotDir = snapshotDir;
            this.manifestPath = manifestPath;
            this.capturedBlocks = capturedBlocks;
            this.fullBlocks = fullBlocks;
            this.partialBlocks = partialBlocks;
            this.fallbackBlocks = fallbackBlocks;
            this.uniqueMeshes = uniqueMeshes;
            this.uniqueTextures = uniqueTextures;
            this.issueSamples = issueSamples;
        }

        public String formatSummaryLine(boolean isRu) {
            if (isRu) {
                return String.format(
                    Locale.ROOT,
                    "Снимок модовых блоков: всего %d (full: %d, partial: %d, fallback: %d), мешей: %d, текстур: %d -> %s",
                    capturedBlocks, fullBlocks, partialBlocks, fallbackBlocks,
                    uniqueMeshes, uniqueTextures, snapshotDir.toAbsolutePath()
                );
            }
            return String.format(
                Locale.ROOT,
                "Modded block snapshot: %d blocks (full: %d, partial: %d, fallback: %d), %d meshes, %d textures -> %s",
                capturedBlocks, fullBlocks, partialBlocks, fallbackBlocks,
                uniqueMeshes, uniqueTextures, snapshotDir.toAbsolutePath()
            );
        }
    }

    /**
     * Pure-Java snapshot package builder and deduplicator.
     * Mesh IDs are assigned AFTER final resolved triangles + materials + tints are known.
     */
    public static final class SnapshotBuilder {
        private final String dimension;
        private final String worldName;
        private final int minX, minY, minZ, maxX, maxY, maxZ;
        private final String capturedAtUtc;

        private final Map<String, TextureEntry> texturesByHash = new LinkedHashMap<>();
        private final Map<String, MeshEntry> meshesByHash = new LinkedHashMap<>();
        private final List<BlockInstanceEntry> blocks = new ArrayList<>();
        private final List<String> issueSamples = new ArrayList<>();

        private int fullCount = 0;
        private int partialCount = 0;
        private int fallbackCount = 0;

        public SnapshotBuilder(
            String dimension,
            String worldName,
            int x1, int y1, int z1,
            int x2, int y2, int z2
        ) {
            this.dimension = (dimension != null && !dimension.isEmpty()) ? dimension : "minecraft:overworld";
            this.worldName = (worldName != null) ? worldName : "";
            this.minX = Math.min(x1, x2);
            this.minY = Math.min(y1, y2);
            this.minZ = Math.min(z1, z2);
            this.maxX = Math.max(x1, x2);
            this.maxY = Math.max(y1, y2);
            this.maxZ = Math.max(z1, z2);
            this.capturedAtUtc = DateTimeFormatter.ISO_INSTANT.format(Instant.now());
        }

        public String registerTexture(String sourceId, int width, int height, byte[] pngBytes) {
            byte[] safeBytes = (pngBytes != null && pngBytes.length > 0) ? pngBytes : fallbackPng16x16();
            String hash = sha256Hex(safeBytes);
            TextureEntry existing = texturesByHash.get(hash);
            if (existing != null) {
                return existing.id;
            }
            String id = String.format(Locale.ROOT, "tex_%04d", texturesByHash.size());
            String relPath = "textures/" + id + ".png";
            String cleanSource = (sourceId != null && !sourceId.isEmpty()) ? sourceId : id;
            int w = Math.max(1, width);
            int h = Math.max(1, height);
            texturesByHash.put(hash, new TextureEntry(id, relPath, cleanSource, w, h, safeBytes));
            return id;
        }

        /**
         * Canonicalizes resolved geometry and materials BEFORE deduplicating mesh instances.
         */
        public String registerMeshIfNonEmpty(List<CapturedTri> triangles) {
            if (triangles == null || triangles.isEmpty()) {
                return null;
            }
            String canonicalKey = computeCanonicalMeshHash(triangles);
            MeshEntry existing = meshesByHash.get(canonicalKey);
            if (existing != null) {
                return existing.id;
            }
            String id = String.format(Locale.ROOT, "mesh_%04d", meshesByHash.size());
            String relPath = "meshes/" + id + ".json";
            List<CapturedTri> copy = new ArrayList<>(triangles.size());
            copy.addAll(triangles);
            meshesByHash.put(canonicalKey, new MeshEntry(id, relPath, copy));
            return id;
        }

        public void addBlock(
            int x, int y, int z,
            String blockId,
            boolean hasBlockEntity,
            CaptureStatus status,
            String captureReason,
            List<CapturedTri> resolvedTriangles,
            List<double[]> collisionBoxes
        ) {
            CaptureStatus effStatus = (status != null) ? status : CaptureStatus.FALLBACK;
            String cleanReason = (captureReason != null && !captureReason.trim().isEmpty())
                ? captureReason.trim()
                : null;
            String meshId = (effStatus == CaptureStatus.FALLBACK)
                ? null
                : registerMeshIfNonEmpty(resolvedTriangles);

            List<double[]> cleanBoxes = new ArrayList<>();
            if (collisionBoxes != null) {
                for (double[] box : collisionBoxes) {
                    if (box != null && box.length == 6
                        && Double.isFinite(box[0]) && Double.isFinite(box[1]) && Double.isFinite(box[2])
                        && Double.isFinite(box[3]) && Double.isFinite(box[4]) && Double.isFinite(box[5])
                        && box[3] > box[0] + 1e-6 && box[4] > box[1] + 1e-6 && box[5] > box[2] + 1e-6) {
                        cleanBoxes.add(Arrays.copyOf(box, 6));
                    }
                }
            }

            switch (effStatus) {
                case FULL -> fullCount++;
                case PARTIAL -> {
                    partialCount++;
                    if (issueSamples.size() < 12) {
                        issueSamples.add(blockId + " @ (" + x + "," + y + "," + z + ") partial"
                            + (cleanReason != null ? ": " + cleanReason : ""));
                    }
                }
                case FALLBACK -> {
                    fallbackCount++;
                    if (issueSamples.size() < 12) {
                        issueSamples.add(blockId + " @ (" + x + "," + y + "," + z + ") fallback"
                            + (cleanReason != null ? ": " + cleanReason : ""));
                    }
                }
            }

            blocks.add(new BlockInstanceEntry(
                x, y, z,
                (blockId != null && !blockId.isEmpty()) ? blockId : "unknown:block",
                hasBlockEntity,
                effStatus,
                cleanReason,
                meshId,
                cleanBoxes
            ));
        }

        public SnapshotResult writeTo(Path outputDir) throws IOException {
            Path absDir = outputDir.toAbsolutePath();
            Path texDir = absDir.resolve("textures");
            Path meshDir = absDir.resolve("meshes");
            Files.createDirectories(texDir);
            Files.createDirectories(meshDir);

            for (TextureEntry tex : texturesByHash.values()) {
                Files.write(absDir.resolve(tex.relPath), tex.pngBytes);
            }
            for (MeshEntry mesh : meshesByHash.values()) {
                Files.writeString(absDir.resolve(mesh.relPath), serializeMeshJson(mesh), StandardCharsets.UTF_8);
            }
            Path manifestPath = absDir.resolve("manifest.json");
            Files.writeString(manifestPath, serializeManifestJson(), StandardCharsets.UTF_8);

            return new SnapshotResult(
                absDir,
                manifestPath,
                blocks.size(),
                fullCount,
                partialCount,
                fallbackCount,
                meshesByHash.size(),
                texturesByHash.size(),
                new ArrayList<>(issueSamples)
            );
        }

        private String serializeManifestJson() {
            StringBuilder sb = new StringBuilder(1024 + blocks.size() * 160);
            sb.append("{\n");
            sb.append("  \"format_version\": ").append(FORMAT_VERSION).append(",\n");
            sb.append("  \"coord_system\": \"").append(COORD_SYSTEM).append("\",\n");
            sb.append("  \"dimension\": ").append(jsonString(dimension)).append(",\n");
            sb.append("  \"world_name\": ").append(jsonString(worldName)).append(",\n");
            sb.append("  \"captured_at_utc\": ").append(jsonString(capturedAtUtc)).append(",\n");
            sb.append("  \"bounds\": {")
              .append("\"min_x\": ").append(minX).append(", ")
              .append("\"min_y\": ").append(minY).append(", ")
              .append("\"min_z\": ").append(minZ).append(", ")
              .append("\"max_x\": ").append(maxX).append(", ")
              .append("\"max_y\": ").append(maxY).append(", ")
              .append("\"max_z\": ").append(maxZ).append("},\n");
            sb.append("  \"summary\": {")
              .append("\"captured_blocks\": ").append(blocks.size()).append(", ")
              .append("\"full_blocks\": ").append(fullCount).append(", ")
              .append("\"partial_blocks\": ").append(partialCount).append(", ")
              .append("\"fallback_blocks\": ").append(fallbackCount).append(", ")
              .append("\"unique_meshes\": ").append(meshesByHash.size()).append(", ")
              .append("\"unique_textures\": ").append(texturesByHash.size()).append("},\n");

            sb.append("  \"textures\": [\n");
            int idx = 0;
            int total = texturesByHash.size();
            for (TextureEntry t : texturesByHash.values()) {
                sb.append("    {\"id\": ").append(jsonString(t.id))
                  .append(", \"path\": ").append(jsonString(t.relPath))
                  .append(", \"source_id\": ").append(jsonString(t.sourceId))
                  .append(", \"width\": ").append(t.width)
                  .append(", \"height\": ").append(t.height).append("}");
                if (++idx < total) sb.append(",");
                sb.append("\n");
            }
            sb.append("  ],\n");

            sb.append("  \"meshes\": [\n");
            idx = 0;
            total = meshesByHash.size();
            for (MeshEntry m : meshesByHash.values()) {
                sb.append("    {\"id\": ").append(jsonString(m.id))
                  .append(", \"path\": ").append(jsonString(m.relPath))
                  .append(", \"triangle_count\": ").append(m.triangles.size()).append("}");
                if (++idx < total) sb.append(",");
                sb.append("\n");
            }
            sb.append("  ],\n");

            sb.append("  \"blocks\": [\n");
            for (int i = 0; i < blocks.size(); i++) {
                BlockInstanceEntry b = blocks.get(i);
                sb.append("    {\"x\": ").append(b.x)
                  .append(", \"y\": ").append(b.y)
                  .append(", \"z\": ").append(b.z)
                  .append(", \"block_id\": ").append(jsonString(b.blockId))
                  .append(", \"has_block_entity\": ").append(b.hasBlockEntity)
                  .append(", \"capture_status\": ").append(jsonString(b.status.wireName))
                  .append(", \"capture_reason\": ").append(b.captureReason != null ? jsonString(b.captureReason) : "null")
                  .append(", \"mesh_id\": ").append(b.meshId != null ? jsonString(b.meshId) : "null")
                  .append(", \"has_collision\": ").append(b.hasCollision)
                  .append(", \"collision_boxes\": [");
                for (int bi = 0; bi < b.collisionBoxes.size(); bi++) {
                    double[] bx = b.collisionBoxes.get(bi);
                    if (bi > 0) sb.append(", ");
                    sb.append("[")
                      .append(fmtDouble(bx[0])).append(", ")
                      .append(fmtDouble(bx[1])).append(", ")
                      .append(fmtDouble(bx[2])).append(", ")
                      .append(fmtDouble(bx[3])).append(", ")
                      .append(fmtDouble(bx[4])).append(", ")
                      .append(fmtDouble(bx[5])).append("]");
                }
                sb.append("]}");
                if (i + 1 < blocks.size()) sb.append(",");
                sb.append("\n");
            }
            sb.append("  ]\n");
            sb.append("}\n");
            return sb.toString();
        }

        private static String serializeMeshJson(MeshEntry mesh) {
            StringBuilder sb = new StringBuilder(128 + mesh.triangles.size() * 180);
            sb.append("{\n");
            sb.append("  \"id\": ").append(jsonString(mesh.id)).append(",\n");
            sb.append("  \"triangles\": [\n");
            for (int i = 0; i < mesh.triangles.size(); i++) {
                CapturedTri t = mesh.triangles.get(i);
                sb.append("    {\"pos\": [")
                  .append(vec3Json(t.pos[0])).append(", ")
                  .append(vec3Json(t.pos[1])).append(", ")
                  .append(vec3Json(t.pos[2])).append("], ")
                  .append("\"normal\": ").append(vec3Json(t.normal)).append(", ")
                  .append("\"uv\": [")
                  .append(vec2Json(t.uv[0])).append(", ")
                  .append(vec2Json(t.uv[1])).append(", ")
                  .append(vec2Json(t.uv[2])).append("], ")
                  .append("\"texture_id\": ").append(jsonString(t.textureId)).append(", ")
                  .append("\"render_layer\": ").append(jsonString(t.renderLayer)).append(", ")
                  .append("\"tint_rgb\": ");
                if (t.tintRgb != null) {
                    sb.append("[").append(t.tintRgb[0]).append(", ").append(t.tintRgb[1]).append(", ").append(t.tintRgb[2]).append("]");
                } else {
                    sb.append("null");
                }
                sb.append("}");
                if (i + 1 < mesh.triangles.size()) sb.append(",");
                sb.append("\n");
            }
            sb.append("  ]\n");
            sb.append("}\n");
            return sb.toString();
        }
    }

    /**
     * Captures all non-vanilla-namespace (or non-vanilla BlockEntity) blocks inside [minPos..maxPos]
     * from the active Minecraft client world. Safe to invoke from any thread; dispatches onto
     * the render thread automatically if needed.
     */
    public static SnapshotResult captureRegion(
        MinecraftClient client,
        BlockPos minPos,
        BlockPos maxPos,
        String dimensionId,
        String worldName,
        Path outputDir
    ) throws Exception {
        if (client == null) {
            throw new IllegalStateException("MinecraftClient is null");
        }
        if (client.isOnThread() || RenderSystem.isOnRenderThread()) {
            return captureOnClientThread(client, minPos, maxPos, dimensionId, worldName, outputDir);
        }
        CompletableFuture<SnapshotResult> future = new CompletableFuture<>();
        client.execute(() -> {
            try {
                future.complete(captureOnClientThread(client, minPos, maxPos, dimensionId, worldName, outputDir));
            } catch (Throwable t) {
                future.completeExceptionally(t);
            }
        });
        try {
            return future.get();
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof Exception ex) throw ex;
            throw new RuntimeException(cause);
        }
    }

    private static SnapshotResult captureOnClientThread(
        MinecraftClient client,
        BlockPos p1,
        BlockPos p2,
        String dimensionId,
        String worldName,
        Path outputDir
    ) throws IOException {
        World world = client.world;
        if (world == null) {
            throw new IllegalStateException("No active client world loaded");
        }

        int minX = Math.min(p1.getX(), p2.getX());
        int minY = Math.min(p1.getY(), p2.getY());
        int minZ = Math.min(p1.getZ(), p2.getZ());
        int maxX = Math.max(p1.getX(), p2.getX());
        int maxY = Math.max(p1.getY(), p2.getY());
        int maxZ = Math.max(p1.getZ(), p2.getZ());

        String resolvedDim = (dimensionId != null && !dimensionId.isEmpty())
            ? dimensionId
            : world.getRegistryKey().getValue().toString();

        SnapshotBuilder builder = new SnapshotBuilder(
            resolvedDim,
            worldName,
            minX, minY, minZ, maxX, maxY, maxZ
        );

        SpriteAtlasTexture blockAtlas = client.getBakedModelManager().getAtlas(SpriteAtlasTexture.BLOCK_ATLAS_TEXTURE);
        SpriteFinder spriteFinder = null;
        try {
            spriteFinder = SpriteFinder.get(blockAtlas);
        } catch (Throwable ignored) {
        }

        TextureExtractor texExtractor = new TextureExtractor(client.getResourceManager(), builder);
        BlockColors blockColors = client.getBlockColors();
        BlockEntityRenderDispatcher beDispatcher = client.getBlockEntityRenderDispatcher();
        Renderer frapiRenderer = RendererAccess.INSTANCE.hasRenderer()
            ? RendererAccess.INSTANCE.getRenderer()
            : null;

        BlockPos.Mutable mutablePos = new BlockPos.Mutable();
        for (int y = minY; y <= maxY; y++) {
            for (int z = minZ; z <= maxZ; z++) {
                for (int x = minX; x <= maxX; x++) {
                    mutablePos.set(x, y, z);
                    BlockState state = world.getBlockState(mutablePos);
                    if (state == null || state.isAir()) {
                        continue;
                    }
                    Identifier blockId = Registries.BLOCK.getId(state.getBlock());
                    BlockEntity be = world.getBlockEntity(mutablePos);
                    if (!shouldCaptureBlock(blockId, be)) {
                        continue;
                    }

                    BlockCaptureOutcome outcome = captureSingleBlock(
                        client,
                        world,
                        mutablePos,
                        state,
                        be,
                        blockColors,
                        beDispatcher,
                        frapiRenderer,
                        spriteFinder,
                        texExtractor
                    );
                    List<double[]> colBoxes = extractCollisionBoxes(world, mutablePos, state);
                    builder.addBlock(
                        x, y, z,
                        blockId.toString(),
                        be != null,
                        outcome.status,
                        outcome.reason,
                        outcome.triangles,
                        colBoxes
                    );
                }
            }
        }

        return builder.writeTo(outputDir);
    }

    static boolean shouldCaptureBlock(Identifier blockId, BlockEntity be) {
        if (blockId != null && !"minecraft".equals(blockId.getNamespace())) {
            return true;
        }
        if (be != null) {
            Identifier beTypeId = BlockEntityType.getId(be.getType());
            return beTypeId != null && !"minecraft".equals(beTypeId.getNamespace());
        }
        return false;
    }

    private static List<double[]> extractCollisionBoxes(BlockRenderView world, BlockPos pos, BlockState state) {
        VoxelShape shape = state.getCollisionShape(world, pos);
        if (shape == null || shape.isEmpty()) {
            return List.of();
        }
        List<Box> boxes = shape.getBoundingBoxes();
        List<double[]> out = new ArrayList<>(boxes.size());
        for (Box b : boxes) {
            out.add(new double[]{b.minX, b.minY, b.minZ, b.maxX, b.maxY, b.maxZ});
        }
        return out;
    }

    private static final class BlockCaptureOutcome {
        final List<CapturedTri> triangles;
        final CaptureStatus status;
        final String reason;

        BlockCaptureOutcome(List<CapturedTri> triangles, CaptureStatus status, String reason) {
            this.triangles = triangles;
            this.status = status;
            this.reason = reason;
        }
    }

    @SuppressWarnings("unchecked")
    private static BlockCaptureOutcome captureSingleBlock(
        MinecraftClient client,
        World world,
        BlockPos pos,
        BlockState state,
        BlockEntity be,
        BlockColors blockColors,
        BlockEntityRenderDispatcher beDispatcher,
        Renderer frapiRenderer,
        SpriteFinder spriteFinder,
        TextureExtractor texExtractor
    ) {
        List<CapturedTri> modelTris = new ArrayList<>();
        List<CapturedTri> berTris = new ArrayList<>();
        List<String> notes = new ArrayList<>();
        boolean modelError = false;
        boolean modelPartial = false;
        boolean modelIsBuiltin = false;

        BakedModel model = null;
        try {
            model = client.getBlockRenderManager().getModel(state);
            if (model != null) {
                modelIsBuiltin = model.isBuiltin();
                Random random = Random.create(state.getRenderingSeed(pos));
                String defaultLayer = renderLayerToBucketName(RenderLayers.getBlockLayer(state));

                boolean handledByFrapi = false;
                if (frapiRenderer != null && model instanceof FabricBakedModel fbm && !fbm.isVanillaAdapter()) {
                    handledByFrapi = true;
                    CapturingRenderContext ctx = new CapturingRenderContext(
                        frapiRenderer, world, pos, state, blockColors, defaultLayer, spriteFinder, texExtractor, modelTris
                    );
                    fbm.emitBlockQuads(world, state, pos, () -> Random.create(state.getRenderingSeed(pos)), ctx);
                }

                if (!handledByFrapi) {
                    emitVanillaBakedModel(
                        model, world, pos, state, random, blockColors, defaultLayer, texExtractor, modelTris
                    );
                }
            }
        } catch (Throwable t) {
            modelError = true;
            notes.add("model capture failed: " + safeMsg(t));
        }

        boolean hasBer = false;
        boolean berUnsupportedDraw = false;
        boolean berFailed = false;
        if (be != null && beDispatcher != null) {
            BlockEntityRenderer<BlockEntity> renderer = beDispatcher.get(be);
            if (renderer != null) {
                hasBer = true;
                try {
                    CapturingVertexConsumerProvider vcp = new CapturingVertexConsumerProvider(
                        state, world, pos, spriteFinder, texExtractor, berTris
                    );
                    MatrixStack stack = new MatrixStack();
                    renderer.render(
                        be,
                        0.0f,
                        stack,
                        vcp,
                        LightmapTextureManager.MAX_LIGHT_COORDINATE,
                        OverlayTexture.DEFAULT_UV
                    );
                    vcp.flushAll();
                    if (vcp.sawUnsupportedDrawMode) {
                        berUnsupportedDraw = true;
                        notes.add("BER used non-triangle DrawMode: " + vcp.unsupportedModes);
                    }
                    if (vcp.sawUnresolvedAtlas) {
                        modelPartial = true;
                        notes.add("BER referenced unresolved texture/atlas");
                    }
                } catch (Throwable t) {
                    berFailed = true;
                    notes.add("BER render error: " + safeMsg(t));
                }
            }
        }

        List<CapturedTri> combined = new ArrayList<>(modelTris.size() + berTris.size());
        combined.addAll(modelTris);
        combined.addAll(berTris);

        if (combined.isEmpty()) {
            if (modelError || berFailed || modelIsBuiltin || hasBer) {
                String reason = notes.isEmpty()
                    ? (modelIsBuiltin ? "Builtin dynamic block model emitted 0 triangles" : "BlockEntityRenderer emitted 0 capturable triangles")
                    : String.join("; ", notes);
                return new BlockCaptureOutcome(combined, CaptureStatus.FALLBACK, reason);
            }
            // Block intentionally has empty geometry (e.g. light/structural block with optional collision)
            return new BlockCaptureOutcome(combined, CaptureStatus.FULL, null);
        }

        if (modelError || berFailed || berUnsupportedDraw || modelPartial) {
            String reason = notes.isEmpty() ? "Partial capture" : String.join("; ", notes);
            return new BlockCaptureOutcome(combined, CaptureStatus.PARTIAL, reason);
        }
        return new BlockCaptureOutcome(combined, CaptureStatus.FULL, null);
    }

    private static void emitVanillaBakedModel(
        BakedModel model,
        BlockRenderView world,
        BlockPos pos,
        BlockState state,
        Random random,
        BlockColors blockColors,
        String defaultLayer,
        TextureExtractor texExtractor,
        List<CapturedTri> outTris
    ) {
        if (model == null) return;
        for (Direction dir : Direction.values()) {
            List<BakedQuad> quads = model.getQuads(state, dir, random);
            if (quads != null) {
                for (BakedQuad bq : quads) {
                    consumeBakedQuad(bq, world, pos, state, blockColors, defaultLayer, texExtractor, outTris);
                }
            }
        }
        List<BakedQuad> unculled = model.getQuads(state, null, random);
        if (unculled != null) {
            for (BakedQuad bq : unculled) {
                consumeBakedQuad(bq, world, pos, state, blockColors, defaultLayer, texExtractor, outTris);
            }
        }
    }

    private static void consumeBakedQuad(
        BakedQuad quad,
        BlockRenderView world,
        BlockPos pos,
        BlockState state,
        BlockColors blockColors,
        String layerName,
        TextureExtractor texExtractor,
        List<CapturedTri> outTris
    ) {
        if (quad == null) return;
        int[] vd = quad.getVertexData();
        if (vd == null || vd.length < 32) return;
        int stride = vd.length / 4;
        if (stride < 6) return;

        Sprite sprite = quad.getSprite();
        String texId = texExtractor.fromSprite(sprite);

        float[][] pts = new float[4][3];
        float[][] uvs = new float[4][2];
        int[][] vertColors = new int[4][3];

        float su0 = sprite != null ? sprite.getMinU() : 0f;
        float su1 = sprite != null ? sprite.getMaxU() : 1f;
        float sv0 = sprite != null ? sprite.getMinV() : 0f;
        float sv1 = sprite != null ? sprite.getMaxV() : 1f;
        float du = su1 - su0;
        float dv = sv1 - sv0;

        for (int vi = 0; vi < 4; vi++) {
            int off = vi * stride;
            pts[vi][0] = Float.intBitsToFloat(vd[off]);
            pts[vi][1] = Float.intBitsToFloat(vd[off + 1]);
            pts[vi][2] = Float.intBitsToFloat(vd[off + 2]);
            int abgr = vd[off + 3];
            // Vertex buffer stores ABGR int on little-endian CPU
            vertColors[vi][0] = abgr & 0xFF;
            vertColors[vi][1] = (abgr >> 8) & 0xFF;
            vertColors[vi][2] = (abgr >> 16) & 0xFF;
            float uRaw = Float.intBitsToFloat(vd[off + 4]);
            float vRaw = Float.intBitsToFloat(vd[off + 5]);
            uvs[vi][0] = (sprite != null && Math.abs(du) > 1e-7f) ? (uRaw - su0) / du : uRaw;
            uvs[vi][1] = (sprite != null && Math.abs(dv) > 1e-7f) ? (vRaw - sv0) / dv : vRaw;
        }

        int[] tint = computeQuadTint(
            world, pos, state, blockColors,
            quad.hasColor() ? quad.getColorIndex() : -1,
            vertColors
        );
        emitQuadTriangles(pts, uvs, null, texId, layerName, tint, outTris);
    }

    private static final class CapturingRenderContext implements RenderContext {
        private final Renderer renderer;
        private final BlockRenderView world;
        private final BlockPos pos;
        private final BlockState state;
        private final BlockColors blockColors;
        private final String defaultLayer;
        private final SpriteFinder spriteFinder;
        private final TextureExtractor texExtractor;
        private final List<CapturedTri> outTris;
        private final Deque<QuadTransform> transformStack = new ArrayDeque<>();

        private final QuadEmitter activeProxyEmitter;
        private MeshBuilder activeBuilder;
        private QuadEmitter activeRawEmitter;

        CapturingRenderContext(
            Renderer renderer,
            BlockRenderView world,
            BlockPos pos,
            BlockState state,
            BlockColors blockColors,
            String defaultLayer,
            SpriteFinder spriteFinder,
            TextureExtractor texExtractor,
            List<CapturedTri> outTris
        ) {
            this.renderer = renderer;
            this.world = world;
            this.pos = pos;
            this.state = state;
            this.blockColors = blockColors;
            this.defaultLayer = defaultLayer;
            this.spriteFinder = spriteFinder;
            this.texExtractor = texExtractor;
            this.outTris = outTris;
            resetActiveEmitter();
            this.activeProxyEmitter = (QuadEmitter) Proxy.newProxyInstance(
                QuadEmitter.class.getClassLoader(),
                new Class<?>[]{QuadEmitter.class},
                (proxy, method, args) -> {
                    QuadEmitter target = this.activeRawEmitter;
                    if ("emit".equals(method.getName()) && (args == null || args.length == 0)) {
                        if (applyTransforms(target)) {
                            consumeQuadView(target);
                        }
                        resetActiveEmitter();
                        return proxy;
                    }
                    Object res = method.invoke(target, args);
                    return (res == target) ? proxy : res;
                }
            );
        }

        private void resetActiveEmitter() {
            this.activeBuilder = renderer.meshBuilder();
            this.activeRawEmitter = activeBuilder.getEmitter();
        }

        private boolean applyTransforms(MutableQuadView quad) {
            if (transformStack.isEmpty()) {
                return true;
            }
            for (QuadTransform qt : transformStack) {
                if (!qt.transform(quad)) {
                    return false;
                }
            }
            return true;
        }

        private void consumeQuadView(QuadView q) {
            Sprite sprite = (spriteFinder != null) ? spriteFinder.find(q) : null;
            String texId = texExtractor.fromSprite(sprite);

            String layerName = defaultLayer;
            RenderMaterial mat = q.material();
            if (mat != null) {
                BlendMode bm = mat.blendMode();
                if (bm == BlendMode.TRANSLUCENT) {
                    layerName = "translucent";
                } else if (bm == BlendMode.CUTOUT || bm == BlendMode.CUTOUT_MIPPED) {
                    layerName = "cutout";
                } else if (bm == BlendMode.SOLID) {
                    layerName = "solid";
                }
            }

            float su0 = sprite != null ? sprite.getMinU() : 0f;
            float su1 = sprite != null ? sprite.getMaxU() : 1f;
            float sv0 = sprite != null ? sprite.getMinV() : 0f;
            float sv1 = sprite != null ? sprite.getMaxV() : 1f;
            float du = su1 - su0;
            float dv = sv1 - sv0;

            float[][] pts = new float[4][3];
            float[][] uvs = new float[4][2];
            int[][] vertColors = new int[4][3];
            for (int vi = 0; vi < 4; vi++) {
                pts[vi][0] = q.x(vi);
                pts[vi][1] = q.y(vi);
                pts[vi][2] = q.z(vi);
                // Fabric QuadView.color(i) is ARGB
                int argb = q.color(vi);
                vertColors[vi][0] = (argb >> 16) & 0xFF;
                vertColors[vi][1] = (argb >> 8) & 0xFF;
                vertColors[vi][2] = argb & 0xFF;
                float uRaw = q.u(vi);
                float vRaw = q.v(vi);
                uvs[vi][0] = (sprite != null && Math.abs(du) > 1e-7f) ? (uRaw - su0) / du : uRaw;
                uvs[vi][1] = (sprite != null && Math.abs(dv) > 1e-7f) ? (vRaw - sv0) / dv : vRaw;
            }

            float[] faceNorm = null;
            try {
                Vector3f fn = q.faceNormal();
                if (fn != null && fn.lengthSquared() > 1e-8f) {
                    faceNorm = new float[]{fn.x, fn.y, fn.z};
                }
            } catch (Throwable ignored) {
            }

            int[] tint = computeQuadTint(world, pos, state, blockColors, q.colorIndex(), vertColors);
            emitQuadTriangles(pts, uvs, faceNorm, texId, layerName, tint, outTris);
        }

        @Override
        public QuadEmitter getEmitter() {
            return activeProxyEmitter;
        }

        @Override
        public boolean hasTransform() {
            return !transformStack.isEmpty();
        }

        @Override
        public void pushTransform(QuadTransform transform) {
            if (transform != null) {
                transformStack.push(transform);
            }
        }

        @Override
        public void popTransform() {
            if (!transformStack.isEmpty()) {
                transformStack.pop();
            }
        }

        @Override
        public boolean isFaceCulled(Direction face) {
            return false;
        }

        @Override
        public Consumer<Mesh> meshConsumer() {
            return mesh -> {
                if (mesh == null) return;
                mesh.forEach(q -> {
                    if (transformStack.isEmpty()) {
                        consumeQuadView(q);
                    } else {
                        q.copyTo(activeRawEmitter);
                        if (applyTransforms(activeRawEmitter)) {
                            consumeQuadView(activeRawEmitter);
                        }
                        resetActiveEmitter();
                    }
                });
            };
        }

        @SuppressWarnings("removal")
        @Override
        public BakedModelConsumer bakedModelConsumer() {
            return new BakedModelConsumer() {
                @Override
                public void accept(BakedModel model) {
                    accept(model, state);
                }

                @Override
                public void accept(BakedModel model, BlockState modelState) {
                    if (model == null) return;
                    BlockState st = (modelState != null) ? modelState : state;
                    Random rng = Random.create(st.getRenderingSeed(pos));
                    for (Direction dir : Direction.values()) {
                        emitQuadList(model.getQuads(st, dir, rng), dir);
                    }
                    emitQuadList(model.getQuads(st, null, rng), null);
                }

                private void emitQuadList(List<BakedQuad> quads, Direction cullFace) {
                    if (quads == null) return;
                    for (BakedQuad bq : quads) {
                        if (transformStack.isEmpty()) {
                            consumeBakedQuad(bq, world, pos, state, blockColors, defaultLayer, texExtractor, outTris);
                        } else {
                            activeRawEmitter.fromVanilla(bq, null, cullFace);
                            if (applyTransforms(activeRawEmitter)) {
                                consumeQuadView(activeRawEmitter);
                            }
                            resetActiveEmitter();
                        }
                    }
                }
            };
        }

        @SuppressWarnings("removal")
        @Override
        public Consumer<BakedModel> fallbackConsumer() {
            return model -> bakedModelConsumer().accept(model);
        }
    }

    private static final class BerVertex {
        float x, y, z;
        float u, v;
        int r = 255, g = 255, b = 255;
    }

    private static final class CapturingVertexConsumerProvider implements VertexConsumerProvider {
        private final BlockState state;
        private final BlockRenderView world;
        private final BlockPos pos;
        private final SpriteFinder spriteFinder;
        private final TextureExtractor texExtractor;
        private final List<CapturedTri> outTris;
        private final Map<RenderLayer, LayerCaptureBuffer> buffers = new LinkedHashMap<>();

        boolean sawUnsupportedDrawMode = false;
        final List<String> unsupportedModes = new ArrayList<>();
        boolean sawUnresolvedAtlas = false;

        CapturingVertexConsumerProvider(
            BlockState state,
            BlockRenderView world,
            BlockPos pos,
            SpriteFinder spriteFinder,
            TextureExtractor texExtractor,
            List<CapturedTri> outTris
        ) {
            this.state = state;
            this.world = world;
            this.pos = pos;
            this.spriteFinder = spriteFinder;
            this.texExtractor = texExtractor;
            this.outTris = outTris;
        }

        @Override
        public VertexConsumer getBuffer(RenderLayer layer) {
            return buffers.computeIfAbsent(layer, LayerCaptureBuffer::new);
        }

        void flushAll() {
            for (LayerCaptureBuffer buf : buffers.values()) {
                buf.flush();
            }
        }

        private final class LayerCaptureBuffer implements VertexConsumer {
            private final RenderLayer layer;
            private final VertexFormat.DrawMode drawMode;
            private final String layerBucket;
            private final Identifier layerTextureId;
            private final boolean usesBlockAtlas;
            private final List<BerVertex> vertices = new ArrayList<>();
            private BerVertex current = new BerVertex();
            private boolean hasFixedColor = false;
            private int fixedR = 255, fixedG = 255, fixedB = 255;

            LayerCaptureBuffer(RenderLayer layer) {
                this.layer = layer;
                this.drawMode = (layer != null && layer.getDrawMode() != null)
                    ? layer.getDrawMode()
                    : VertexFormat.DrawMode.QUADS;
                this.layerBucket = renderLayerToBucketName(layer);
                this.layerTextureId = extractTextureIdFromRenderLayer(layer);
                this.usesBlockAtlas = (this.layerTextureId == null)
                    || SpriteAtlasTexture.BLOCK_ATLAS_TEXTURE.equals(this.layerTextureId)
                    || "textures/atlas/blocks.png".equals(this.layerTextureId.getPath());
            }

            @Override
            public VertexConsumer vertex(double x, double y, double z) {
                current.x = (float) x;
                current.y = (float) y;
                current.z = (float) z;
                return this;
            }

            @Override
            public VertexConsumer color(int red, int green, int blue, int alpha) {
                if (!hasFixedColor) {
                    current.r = clamp255(red);
                    current.g = clamp255(green);
                    current.b = clamp255(blue);
                }
                return this;
            }

            @Override
            public VertexConsumer texture(float u, float v) {
                current.u = u;
                current.v = v;
                return this;
            }

            @Override
            public VertexConsumer overlay(int u, int v) {
                return this;
            }

            @Override
            public VertexConsumer light(int u, int v) {
                return this;
            }

            @Override
            public VertexConsumer normal(float x, float y, float z) {
                return this;
            }

            @Override
            public void next() {
                if (hasFixedColor) {
                    current.r = fixedR;
                    current.g = fixedG;
                    current.b = fixedB;
                }
                vertices.add(current);
                current = new BerVertex();
            }

            @Override
            public void fixedColor(int red, int green, int blue, int alpha) {
                hasFixedColor = true;
                fixedR = clamp255(red);
                fixedG = clamp255(green);
                fixedB = clamp255(blue);
            }

            @Override
            public void unfixColor() {
                hasFixedColor = false;
            }

            @Override
            public void vertex(
                float x, float y, float z,
                float red, float green, float blue, float alpha,
                float u, float v,
                int overlay, int light,
                float normalX, float normalY, float normalZ
            ) {
                current.x = x;
                current.y = y;
                current.z = z;
                current.r = clamp255(Math.round(red * 255.0f));
                current.g = clamp255(Math.round(green * 255.0f));
                current.b = clamp255(Math.round(blue * 255.0f));
                current.u = u;
                current.v = v;
                next();
            }

            void flush() {
                if (vertices.isEmpty()) return;
                switch (drawMode) {
                    case QUADS -> {
                        int count = (vertices.size() / 4) * 4;
                        for (int i = 0; i + 3 < count; i += 4) {
                            emitBerQuad(vertices.get(i), vertices.get(i + 1), vertices.get(i + 2), vertices.get(i + 3));
                        }
                    }
                    case TRIANGLES -> {
                        int count = (vertices.size() / 3) * 3;
                        for (int i = 0; i + 2 < count; i += 3) {
                            emitBerTri(vertices.get(i), vertices.get(i + 1), vertices.get(i + 2));
                        }
                    }
                    case TRIANGLE_STRIP -> {
                        for (int i = 2; i < vertices.size(); i++) {
                            if ((i & 1) == 0) {
                                emitBerTri(vertices.get(i - 2), vertices.get(i - 1), vertices.get(i));
                            } else {
                                emitBerTri(vertices.get(i - 1), vertices.get(i - 2), vertices.get(i));
                            }
                        }
                    }
                    case TRIANGLE_FAN -> {
                        for (int i = 2; i < vertices.size(); i++) {
                            emitBerTri(vertices.get(0), vertices.get(i - 1), vertices.get(i));
                        }
                    }
                    default -> {
                        sawUnsupportedDrawMode = true;
                        String m = drawMode.name();
                        if (!unsupportedModes.contains(m)) {
                            unsupportedModes.add(m);
                        }
                    }
                }
                vertices.clear();
            }

            private void emitBerQuad(BerVertex a, BerVertex b, BerVertex c, BerVertex d) {
                emitBerTri(a, b, c);
                emitBerTri(a, c, d);
            }

            private void emitBerTri(BerVertex v0, BerVertex v1, BerVertex v2) {
                float[][] pts = {
                    {v0.x, v0.y, v0.z},
                    {v1.x, v1.y, v1.z},
                    {v2.x, v2.y, v2.z}
                };
                float[] norm = computeTriangleNormal(pts[0], pts[1], pts[2]);
                if (norm == null) return;

                String texId;
                float[][] uvs = new float[3][2];
                if (usesBlockAtlas) {
                    float cu = (v0.u + v1.u + v2.u) / 3.0f;
                    float cv = (v0.v + v1.v + v2.v) / 3.0f;
                    Sprite sprite = (spriteFinder != null) ? spriteFinder.find(cu, cv) : null;
                    if (sprite != null) {
                        texId = texExtractor.fromSprite(sprite);
                        float su0 = sprite.getMinU(), su1 = sprite.getMaxU();
                        float sv0 = sprite.getMinV(), sv1 = sprite.getMaxV();
                        float du = su1 - su0, dv = sv1 - sv0;
                        uvs[0][0] = Math.abs(du) > 1e-7f ? (v0.u - su0) / du : v0.u;
                        uvs[0][1] = Math.abs(dv) > 1e-7f ? (v0.v - sv0) / dv : v0.v;
                        uvs[1][0] = Math.abs(du) > 1e-7f ? (v1.u - su0) / du : v1.u;
                        uvs[1][1] = Math.abs(dv) > 1e-7f ? (v1.v - sv0) / dv : v1.v;
                        uvs[2][0] = Math.abs(du) > 1e-7f ? (v2.u - su0) / du : v2.u;
                        uvs[2][1] = Math.abs(dv) > 1e-7f ? (v2.v - sv0) / dv : v2.v;
                    } else {
                        sawUnresolvedAtlas = true;
                        texId = texExtractor.missingTextureId();
                        uvs[0][0] = v0.u; uvs[0][1] = v0.v;
                        uvs[1][0] = v1.u; uvs[1][1] = v1.v;
                        uvs[2][0] = v2.u; uvs[2][1] = v2.v;
                    }
                } else {
                    texId = texExtractor.fromResourceIdentifier(layerTextureId);
                    uvs[0][0] = v0.u; uvs[0][1] = v0.v;
                    uvs[1][0] = v1.u; uvs[1][1] = v1.v;
                    uvs[2][0] = v2.u; uvs[2][1] = v2.v;
                }

                int r = (v0.r + v1.r + v2.r) / 3;
                int g = (v0.g + v1.g + v2.g) / 3;
                int b = (v0.b + v1.b + v2.b) / 3;
                int[] tint = (r == 255 && g == 255 && b == 255) ? null : new int[]{r, g, b};
                outTris.add(new CapturedTri(pts, norm, uvs, texId, layerBucket, tint));
            }
        }
    }

    private static final class TextureExtractor {
        private final ResourceManager resourceManager;
        private final SnapshotBuilder builder;
        private final Map<String, String> cacheBySourceKey = new HashMap<>();
        private String missingId = null;

        TextureExtractor(ResourceManager resourceManager, SnapshotBuilder builder) {
            this.resourceManager = resourceManager;
            this.builder = builder;
        }

        String missingTextureId() {
            if (missingId == null) {
                missingId = builder.registerTexture("minecraft:missing", 16, 16, fallbackPng16x16());
            }
            return missingId;
        }

        String fromSprite(Sprite sprite) {
            if (sprite == null || sprite.getContents() == null) {
                return missingTextureId();
            }
            SpriteContents contents = sprite.getContents();
            Identifier spriteId = contents.getId();
            String cacheKey = "sprite:" + spriteId.toString();
            String cached = cacheBySourceKey.get(cacheKey);
            if (cached != null) return cached;

            int frameW = Math.max(1, contents.getWidth());
            int frameH = Math.max(1, contents.getHeight());
            byte[] pngBytes = tryExtractSpritePng(contents, spriteId, frameW, frameH);
            if (pngBytes == null) {
                pngBytes = fallbackPng16x16();
            }
            String texId = builder.registerTexture(spriteId.toString(), frameW, frameH, pngBytes);
            cacheBySourceKey.put(cacheKey, texId);
            return texId;
        }

        String fromResourceIdentifier(Identifier texResourceId) {
            if (texResourceId == null) {
                return missingTextureId();
            }
            String cacheKey = "res:" + texResourceId.toString();
            String cached = cacheBySourceKey.get(cacheKey);
            if (cached != null) return cached;

            byte[] pngBytes = null;
            int w = 16, h = 16;
            if (resourceManager != null) {
                Optional<Resource> resOpt = resourceManager.getResource(texResourceId);
                if (resOpt.isPresent()) {
                    try (InputStream is = resOpt.get().getInputStream();
                         NativeImage img = NativeImage.read(is)) {
                        w = Math.max(1, img.getWidth());
                        h = Math.max(1, img.getHeight());
                        pngBytes = img.getBytes();
                    } catch (Throwable ignored) {
                    }
                }
            }
            if (pngBytes == null) {
                pngBytes = fallbackPng16x16();
            }
            String texId = builder.registerTexture(texResourceId.toString(), w, h, pngBytes);
            cacheBySourceKey.put(cacheKey, texId);
            return texId;
        }

        private byte[] tryExtractSpritePng(SpriteContents contents, Identifier spriteId, int frameW, int frameH) {
            // 1. Prefer reading runtime SpriteContents mip-0 image (reflects dynamic/atlas-generated textures too)
            NativeImage runtimeImg = findSpriteBaseImage(contents);
            if (runtimeImg != null) {
                byte[] cropped = cropFirstFramePng(runtimeImg, frameW, frameH);
                if (cropped != null) return cropped;
            }
            // 2. Fall back to active resource pack PNG at textures/<path>.png
            if (resourceManager != null && spriteId != null) {
                Identifier texPath = new Identifier(spriteId.getNamespace(), "textures/" + spriteId.getPath() + ".png");
                Optional<Resource> resOpt = resourceManager.getResource(texPath);
                if (resOpt.isPresent()) {
                    try (InputStream is = resOpt.get().getInputStream();
                         NativeImage img = NativeImage.read(is)) {
                        byte[] cropped = cropFirstFramePng(img, frameW, frameH);
                        if (cropped != null) return cropped;
                    } catch (Throwable ignored) {
                    }
                }
            }
            return null;
        }

        /**
         * Scans SpriteContents by field type rather than Yarn field name so it works identically
         * in both named dev environments and tiny-remapped intermediary production jars.
         */
        private static NativeImage findSpriteBaseImage(SpriteContents contents) {
            for (Field f : SpriteContents.class.getDeclaredFields()) {
                if (f.getType() == NativeImage[].class) {
                    try {
                        f.setAccessible(true);
                        NativeImage[] mips = (NativeImage[]) f.get(contents);
                        if (mips != null && mips.length > 0 && mips[0] != null) {
                            return mips[0];
                        }
                    } catch (Throwable ignored) {
                    }
                }
            }
            for (Field f : SpriteContents.class.getDeclaredFields()) {
                if (f.getType() == NativeImage.class) {
                    try {
                        f.setAccessible(true);
                        NativeImage img = (NativeImage) f.get(contents);
                        if (img != null) {
                            return img;
                        }
                    } catch (Throwable ignored) {
                    }
                }
            }
            return null;
        }

        private static byte[] cropFirstFramePng(NativeImage src, int frameW, int frameH) {
            int sw = src.getWidth();
            int sh = src.getHeight();
            if (sw <= 0 || sh <= 0) return null;
            int w = Math.min(frameW, sw);
            int h = Math.min(frameH, sh);
            if (w == sw && h == sh) {
                try {
                    return src.getBytes();
                } catch (Throwable ignored) {
                    return null;
                }
            }
            try (NativeImage cropped = new NativeImage(w, h, false)) {
                for (int y = 0; y < h; y++) {
                    for (int x = 0; x < w; x++) {
                        cropped.setColor(x, y, src.getColor(x, y));
                    }
                }
                return cropped.getBytes();
            } catch (Throwable ignored) {
                return null;
            }
        }
    }

    private static int[] computeQuadTint(
        BlockRenderView world,
        BlockPos pos,
        BlockState state,
        BlockColors blockColors,
        int colorIndex,
        int[][] vertColors
    ) {
        int vr = (vertColors[0][0] + vertColors[1][0] + vertColors[2][0] + vertColors[3][0]) / 4;
        int vg = (vertColors[0][1] + vertColors[1][1] + vertColors[2][1] + vertColors[3][1]) / 4;
        int vb = (vertColors[0][2] + vertColors[1][2] + vertColors[2][2] + vertColors[3][2]) / 4;

        if (colorIndex >= 0 && blockColors != null) {
            int bc = blockColors.getColor(state, world, pos, colorIndex);
            if (bc != -1) {
                int br = (bc >> 16) & 0xFF;
                int bg = (bc >> 8) & 0xFF;
                int bb = bc & 0xFF;
                vr = (vr * br + 127) / 255;
                vg = (vg * bg + 127) / 255;
                vb = (vb * bb + 127) / 255;
            }
        }

        if (vr >= 255 && vg >= 255 && vb >= 255) {
            return null;
        }
        return new int[]{clamp255(vr), clamp255(vg), clamp255(vb)};
    }

    private static void emitQuadTriangles(
        float[][] pts,
        float[][] uvs,
        float[] overrideNormal,
        String texId,
        String layerName,
        int[] tint,
        List<CapturedTri> outTris
    ) {
        emitSingleTri(pts[0], pts[1], pts[2], uvs[0], uvs[1], uvs[2], overrideNormal, texId, layerName, tint, outTris);
        emitSingleTri(pts[0], pts[2], pts[3], uvs[0], uvs[2], uvs[3], overrideNormal, texId, layerName, tint, outTris);
    }

    private static void emitSingleTri(
        float[] p0, float[] p1, float[] p2,
        float[] uv0, float[] uv1, float[] uv2,
        float[] overrideNormal,
        String texId,
        String layerName,
        int[] tint,
        List<CapturedTri> outTris
    ) {
        float[] norm = computeTriangleNormal(p0, p1, p2);
        if (norm == null) {
            return;
        }
        if (overrideNormal != null && overrideNormal.length == 3) {
            float lenSq = overrideNormal[0] * overrideNormal[0]
                + overrideNormal[1] * overrideNormal[1]
                + overrideNormal[2] * overrideNormal[2];
            if (lenSq > 1e-8f) {
                float len = (float) Math.sqrt(lenSq);
                norm = new float[]{overrideNormal[0] / len, overrideNormal[1] / len, overrideNormal[2] / len};
            }
        }
        outTris.add(new CapturedTri(
            new float[][]{p0, p1, p2},
            norm,
            new float[][]{uv0, uv1, uv2},
            texId,
            layerName,
            tint
        ));
    }

    private static float[] computeTriangleNormal(float[] p0, float[] p1, float[] p2) {
        float ux = p1[0] - p0[0], uy = p1[1] - p0[1], uz = p1[2] - p0[2];
        float vx = p2[0] - p0[0], vy = p2[1] - p0[1], vz = p2[2] - p0[2];
        float nx = uy * vz - uz * vy;
        float ny = uz * vx - ux * vz;
        float nz = ux * vy - uy * vx;
        float lenSq = nx * nx + ny * ny + nz * nz;
        if (lenSq < 1e-12f || !Float.isFinite(lenSq)) {
            return null;
        }
        float inv = 1.0f / (float) Math.sqrt(lenSq);
        return new float[]{nx * inv, ny * inv, nz * inv};
    }

    private static String renderLayerToBucketName(RenderLayer layer) {
        if (layer == null) return "solid";
        if (layer == RenderLayer.getTranslucent()
            || layer == RenderLayer.getTranslucentMovingBlock()
            || layer == RenderLayer.getTranslucentNoCrumbling()) {
            return "translucent";
        }
        if (layer == RenderLayer.getCutout() || layer == RenderLayer.getCutoutMipped()) {
            return "cutout";
        }
        if (layer == RenderLayer.getSolid()) {
            return "solid";
        }
        String s = layer.toString().toLowerCase(Locale.ROOT);
        if (s.contains("translucent") || s.contains("water")) return "translucent";
        if (s.contains("cutout")) return "cutout";
        return "solid";
    }

    private static Identifier extractTextureIdFromRenderLayer(RenderLayer layer) {
        if (layer == null) return null;
        String desc = layer.toString();
        if (desc != null) {
            Matcher m = LAYER_TEX_PATTERN.matcher(desc);
            if (m.find()) {
                Identifier parsed = Identifier.tryParse(m.group(1));
                if (parsed != null) return parsed;
            }
        }
        // Fallback: inspect MultiPhaseParameters -> RenderPhase$TextureBase -> Optional<Identifier> by field type
        try {
            for (Field f : layer.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                Object val = f.get(layer);
                if (val == null) continue;
                for (Field pf : val.getClass().getDeclaredFields()) {
                    pf.setAccessible(true);
                    Object pv = pf.get(val);
                    if (pv == null) continue;
                    for (Field tf : pv.getClass().getDeclaredFields()) {
                        if (tf.getType() == Optional.class) {
                            tf.setAccessible(true);
                            Object optObj = tf.get(pv);
                            if (optObj instanceof Optional<?> opt && opt.isPresent() && opt.get() instanceof Identifier id) {
                                return id;
                            }
                        }
                    }
                }
            }
        } catch (Throwable ignored) {
        }
        return null;
    }

    private static String computeCanonicalMeshHash(List<CapturedTri> tris) {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            byte[] lenBuf = intBytes(tris.size());
            md.update(lenBuf);
            for (CapturedTri t : tris) {
                for (int i = 0; i < 3; i++) {
                    md.update(intBytes(quantizedBits(t.pos[i][0])));
                    md.update(intBytes(quantizedBits(t.pos[i][1])));
                    md.update(intBytes(quantizedBits(t.pos[i][2])));
                    md.update(intBytes(quantizedBits(t.uv[i][0])));
                    md.update(intBytes(quantizedBits(t.uv[i][1])));
                }
                md.update(intBytes(quantizedBits(t.normal[0])));
                md.update(intBytes(quantizedBits(t.normal[1])));
                md.update(intBytes(quantizedBits(t.normal[2])));
                updateStr(md, t.textureId);
                updateStr(md, t.renderLayer);
                if (t.tintRgb != null) {
                    md.update((byte) 1);
                    md.update((byte) t.tintRgb[0]);
                    md.update((byte) t.tintRgb[1]);
                    md.update((byte) t.tintRgb[2]);
                } else {
                    md.update((byte) 0);
                }
            }
            return toHex(md.digest());
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    private static void updateStr(MessageDigest md, String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        md.update(intBytes(b.length));
        md.update(b);
    }

    private static int quantizedBits(float v) {
        if (!Float.isFinite(v)) return 0;
        float q = Math.round(v * 100000.0f) / 100000.0f;
        if (q == 0.0f) q = 0.0f;
        return Float.floatToIntBits(q);
    }

    private static byte[] intBytes(int v) {
        return new byte[]{
            (byte) (v >>> 24),
            (byte) (v >>> 16),
            (byte) (v >>> 8),
            (byte) v
        };
    }

    private static String sha256Hex(byte[] data) {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            return toHex(md.digest(data));
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String toHex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16));
            sb.append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    private static byte[] fallbackPng16x16() {
        // Minimal valid 16x16 solid magenta/black 1-color PNG generated lazily or static bytes?
        // Using BufferedImage/ImageIO avoids NativeImage native library dependency in headless unit tests.
        try {
            java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(16, 16, java.awt.image.BufferedImage.TYPE_INT_ARGB);
            for (int y = 0; y < 16; y++) {
                for (int x = 0; x < 16; x++) {
                    boolean pink = ((x / 8) ^ (y / 8)) == 0;
                    img.setRGB(x, y, pink ? 0xFFF800F8 : 0xFF000000);
                }
            }
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            javax.imageio.ImageIO.write(img, "PNG", baos);
            return baos.toByteArray();
        } catch (Throwable ignored) {
            return new byte[0];
        }
    }

    private static String normalizeLayerName(String s) {
        if ("translucent".equals(s) || "cutout".equals(s)) return s;
        return "solid";
    }

    private static int clamp255(int v) {
        return Math.max(0, Math.min(255, v));
    }

    private static String safeMsg(Throwable t) {
        String m = t.getMessage();
        return (m != null && !m.isEmpty()) ? m : t.getClass().getSimpleName();
    }

    private static String vec3Json(float[] v) {
        return "[" + fmtFloat(v[0]) + ", " + fmtFloat(v[1]) + ", " + fmtFloat(v[2]) + "]";
    }

    private static String vec2Json(float[] v) {
        return "[" + fmtFloat(v[0]) + ", " + fmtFloat(v[1]) + "]";
    }

    private static String fmtFloat(float v) {
        if (!Float.isFinite(v)) return "0.0";
        if (Math.abs(v) < 1e-7f) return "0.0";
        return String.format(Locale.ROOT, "%.6f", v).replaceAll("0+$", "").replaceAll("\\.$", ".0");
    }

    private static String fmtDouble(double v) {
        if (!Double.isFinite(v)) return "0.0";
        if (Math.abs(v) < 1e-7) return "0.0";
        return String.format(Locale.ROOT, "%.6f", v).replaceAll("0+$", "").replaceAll("\\.$", ".0");
    }

    private static String jsonString(String s) {
        if (s == null) return "null";
        StringBuilder sb = new StringBuilder(s.length() + 4);
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> sb.append("\\\"");
                case '\\' -> sb.append("\\\\");
                case '\b' -> sb.append("\\b");
                case '\f' -> sb.append("\\f");
                case '\n' -> sb.append("\\n");
                case '\r' -> sb.append("\\r");
                case '\t' -> sb.append("\\t");
                default -> {
                    if (c < 0x20) {
                        sb.append(String.format(Locale.ROOT, "\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
                }
            }
        }
        sb.append('"');
        return sb.toString();
    }
}
