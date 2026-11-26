package com.mcgm.bridge;

import net.minecraft.client.MinecraftClient;
import net.minecraft.text.Text;
import net.minecraft.util.hit.BlockHitResult;
import net.minecraft.util.hit.HitResult;
import net.minecraft.util.math.BlockPos;

/**
 * Manages the current 3D selection points (pos1 and pos2) in the Minecraft world.
 */
public class SelectionManager {
    private static final SelectionManager INSTANCE = new SelectionManager();

    private BlockPos pos1 = null;
    private BlockPos pos2 = null;
    private BlockPos cachedMinPos = null;
    private BlockPos cachedMaxPos = null;
    private String cachedDimText = "Pos 1 (Waiting for Pos 2)";

    public static SelectionManager getInstance() {
        return INSTANCE;
    }

    private SelectionManager() {}

    private void recomputeCache() {
        if (pos1 != null && pos2 != null) {
            cachedMinPos = new BlockPos(
                Math.min(pos1.getX(), pos2.getX()),
                Math.min(pos1.getY(), pos2.getY()),
                Math.min(pos1.getZ(), pos2.getZ())
            );
            cachedMaxPos = new BlockPos(
                Math.max(pos1.getX(), pos2.getX()),
                Math.max(pos1.getY(), pos2.getY()),
                Math.max(pos1.getZ(), pos2.getZ())
            );
            long dx = (long) cachedMaxPos.getX() - (long) cachedMinPos.getX() + 1L;
            long dy = (long) cachedMaxPos.getY() - (long) cachedMinPos.getY() + 1L;
            long dz = (long) cachedMaxPos.getZ() - (long) cachedMinPos.getZ() + 1L;
            long count = dx * dy * dz;
            cachedDimText = dx + " × " + dy + " × " + dz + " (" + count + " blocks)";
        } else if (pos1 != null) {
            cachedMinPos = pos1;
            cachedMaxPos = pos1;
            cachedDimText = "Pos 1 (Waiting for Pos 2)";
        } else if (pos2 != null) {
            cachedMinPos = pos2;
            cachedMaxPos = pos2;
            cachedDimText = "Pos 2 (Waiting for Pos 1)";
        } else {
            cachedMinPos = null;
            cachedMaxPos = null;
            cachedDimText = "No selection";
        }
    }

    public synchronized BlockPos getPos1() {
        return pos1;
    }

    public synchronized BlockPos getPos2() {
        return pos2;
    }

    public synchronized void setPos1(BlockPos p) {
        this.pos1 = p;
        recomputeCache();
    }

    public synchronized void setPos2(BlockPos p) {
        this.pos2 = p;
        recomputeCache();
    }

    public synchronized void clear() {
        this.pos1 = null;
        this.pos2 = null;
        recomputeCache();
    }

    public synchronized boolean hasSelection() {
        return pos1 != null || pos2 != null;
    }

    public synchronized BlockPos getMinPos() {
        return cachedMinPos;
    }

    public synchronized BlockPos getMaxPos() {
        return cachedMaxPos;
    }

    public synchronized String getDimText() {
        return cachedDimText;
    }

    public synchronized void setCoords(int x1, int y1, int z1, int x2, int y2, int z2) {
        this.pos1 = new BlockPos(Math.min(x1, x2), Math.min(y1, y2), Math.min(z1, z2));
        this.pos2 = new BlockPos(Math.max(x1, x2), Math.max(y1, y2), Math.max(z1, z2));
        recomputeCache();
    }

    public synchronized void setPos1FromCommand(MinecraftClient client, BlockPos pos) {
        if (pos == null) return;
        setPos1(pos);
        notifyPos(client, "Pos 1", pos, false, true);
    }

    public synchronized void setPos2FromCommand(MinecraftClient client, BlockPos pos) {
        if (pos == null) return;
        setPos2(pos);
        notifyPos(client, "Pos 2", pos, false, true);
    }

    public void setPos1FromPlayer(MinecraftClient client) {
        if (client.player == null) return;
        BlockPos target = getTargetBlock(client);
        boolean fromCrosshair = (target != null);
        if (target == null) {
            target = client.player.getBlockPos();
        }
        setPos1(target);
        notifyPos(client, "Pos 1", target, fromCrosshair, false);
    }

    public void setPos2FromPlayer(MinecraftClient client) {
        if (client.player == null) return;
        BlockPos target = getTargetBlock(client);
        boolean fromCrosshair = (target != null);
        if (target == null) {
            target = client.player.getBlockPos();
        }
        setPos2(target);
        notifyPos(client, "Pos 2", target, fromCrosshair, false);
    }

    private BlockPos getTargetBlock(MinecraftClient client) {
        if (client.player == null || client.world == null) return null;
        net.minecraft.entity.Entity entity = client.getCameraEntity() != null ? client.getCameraEntity() : client.player;
        // Long-range raycast up to 512 blocks from player view camera, including fluids (water/lava)
        HitResult hit = entity.raycast(512.0, 1.0f, true);
        if (hit != null && hit.getType() == HitResult.Type.BLOCK && hit instanceof BlockHitResult bhr) {
            return bhr.getBlockPos();
        }
        // Fallback to vanilla crosshair target if available
        if (client.crosshairTarget instanceof BlockHitResult bhr2 && bhr2.getType() == HitResult.Type.BLOCK) {
            return bhr2.getBlockPos();
        }
        return null;
    }

    private void notifyPos(MinecraftClient client, String label, BlockPos pos, boolean fromCrosshair, boolean fromCommand) {
        if (client == null || client.player == null) return;
        net.minecraft.entity.Entity entity = client.getCameraEntity() != null ? client.getCameraEntity() : client.player;
        double dist = Math.sqrt(entity.squaredDistanceTo(pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5));
        String targetDesc;
        if (fromCommand) {
            String blockSuffix = (client.world != null)
                ? client.world.getBlockState(pos).getBlock().getName().getString() + ", "
                : "";
            targetDesc = " §7(" + blockSuffix + "command)§r";
        } else if (fromCrosshair && client.world != null) {
            targetDesc = " §7(" + client.world.getBlockState(pos).getBlock().getName().getString() + ", " + String.format("%.1f", dist) + "m)§r";
        } else {
            targetDesc = " §e(at player position)§r";
        }
        client.player.sendMessage(
            Text.literal("§6[MCGM]§r " + label + " set to §b(" + pos.getX() + ", " + pos.getY() + ", " + pos.getZ() + ")§r" + targetDesc),
            false
        );
        if (pos1 != null && pos2 != null) {
            int dx = Math.abs(pos2.getX() - pos1.getX()) + 1;
            int dy = Math.abs(pos2.getY() - pos1.getY()) + 1;
            int dz = Math.abs(pos2.getZ() - pos1.getZ()) + 1;
            long volume = (long) dx * dy * dz;
            client.player.sendMessage(
                Text.literal("§6[MCGM]§a Selection size: §f" + dx + " × " + dy + " × " + dz + "§a blocks (§e" + volume + "§a total)"),
                false
            );
        }
    }
}
