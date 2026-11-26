package com.mcgm.bridge;

import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderContext;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderEvents;
import net.minecraft.client.render.Camera;
import net.minecraft.client.render.RenderLayer;
import net.minecraft.client.render.VertexConsumer;
import net.minecraft.client.render.VertexConsumerProvider;
import net.minecraft.client.render.WorldRenderer;
import net.minecraft.client.render.debug.DebugRenderer;
import net.minecraft.client.util.math.MatrixStack;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Vec3d;

/**
 * Renders the semi-transparent 3D wireframe bounding box around the selected region
 * with dimensions floating in blocks.
 */
public class BoundingBoxRenderer {

    public static void register() {
        WorldRenderEvents.BEFORE_DEBUG_RENDER.register(BoundingBoxRenderer::render);
    }

    private static void render(WorldRenderContext context) {
        SelectionManager sm = SelectionManager.getInstance();
        if (!sm.hasSelection()) {
            return;
        }

        BlockPos min = sm.getMinPos();
        BlockPos max = sm.getMaxPos();
        if (min == null || max == null) {
            return;
        }

        Camera camera = context.camera();
        if (camera == null) {
            return;
        }
        Vec3d camPos = camera.getPos();

        // Exact block bounds: from min to max + 1.0 block
        double minX = Math.min(min.getX(), max.getX());
        double minY = Math.min(min.getY(), max.getY());
        double minZ = Math.min(min.getZ(), max.getZ());
        double maxX = Math.max(min.getX(), max.getX()) + 1.0;
        double maxY = Math.max(min.getY(), max.getY()) + 1.0;
        double maxZ = Math.max(min.getZ(), max.getZ()) + 1.0;

        // Camera-relative bounding box coordinates
        double x1 = minX - camPos.x;
        double y1 = minY - camPos.y;
        double z1 = minZ - camPos.z;
        double x2 = maxX - camPos.x;
        double y2 = maxY - camPos.y;
        double z2 = maxZ - camPos.z;

        MatrixStack matrices = context.matrixStack();
        VertexConsumerProvider consumers = context.consumers();
        if (consumers == null || matrices == null) {
            return;
        }

        matrices.push();

        boolean hasBoth = (sm.getPos1() != null && sm.getPos2() != null);

        // 1. Semi-transparent filled faces
        VertexConsumer fillConsumer = consumers.getBuffer(RenderLayer.getDebugFilledBox());
        if (hasBoth) {
            WorldRenderer.renderFilledBox(matrices, fillConsumer, x1, y1, z1, x2, y2, z2, 0.0f, 0.75f, 1.0f, 0.18f);
        } else {
            // Single point marker: yellow highlight
            WorldRenderer.renderFilledBox(matrices, fillConsumer, x1, y1, z1, x2, y2, z2, 1.0f, 0.85f, 0.0f, 0.35f);
        }

        // 2. Wireframe bounding box edges
        VertexConsumer lineConsumer = consumers.getBuffer(RenderLayer.getLines());
        if (hasBoth) {
            WorldRenderer.drawBox(matrices, lineConsumer, x1, y1, z1, x2, y2, z2, 0.1f, 1.0f, 0.85f, 1.0f);
        } else {
            WorldRenderer.drawBox(matrices, lineConsumer, x1, y1, z1, x2, y2, z2, 1.0f, 0.9f, 0.1f, 1.0f);
        }

        // 3. Floating dimension text above center of selection box
        String dimText = sm.getDimText();

        double centerX = (minX + maxX) / 2.0;
        double topY = maxY + 0.4;
        double centerZ = (minZ + maxZ) / 2.0;

        int textColor = hasBoth ? 0xFF00FFCC : 0xFFFFD700;
        DebugRenderer.drawString(matrices, consumers, dimText, centerX, topY, centerZ, textColor, 0.035f, true, 0.0f, true);

        matrices.pop();
    }
}
