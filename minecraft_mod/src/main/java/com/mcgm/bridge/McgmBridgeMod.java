package com.mcgm.bridge;

import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.arguments.ArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import com.mojang.brigadier.suggestion.Suggestions;
import com.mojang.brigadier.suggestion.SuggestionsBuilder;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandManager;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.command.v2.FabricClientCommandSource;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.keybinding.v1.KeyBindingHelper;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.option.KeyBinding;
import net.minecraft.command.argument.BlockPosArgumentType;
import net.minecraft.command.argument.CoordinateArgument;
import net.minecraft.command.argument.Vec3ArgumentType;
import net.minecraft.text.Text;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Vec3d;
import org.lwjgl.glfw.GLFW;

import java.util.Arrays;
import java.util.Collection;
import java.util.concurrent.CompletableFuture;

/**
 * Main ClientModInitializer entrypoint for the mcgm-bridge Fabric 1.20.1 mod.
 */
public class McgmBridgeMod implements ClientModInitializer {

    public static final String MOD_ID = "mcgm-bridge";

    private static KeyBinding keyPos1;
    private static KeyBinding keyPos2;
    private static KeyBinding keyExportScreen;
    private static volatile net.minecraft.client.gui.screen.Screen pendingScreen = null;

    public static void openScreenNextTick(net.minecraft.client.gui.screen.Screen screen) {
        pendingScreen = screen;
    }

    @Override
    public void onInitializeClient() {
        // 1. Register Keybindings (Category: MCGM Bridge to Garry's Mod)
        keyPos1 = KeyBindingHelper.registerKeyBinding(new KeyBinding(
            "key.mcgm.pos1",
            GLFW.GLFW_KEY_LEFT_BRACKET,
            "key.categories.mcgm"
        ));

        keyPos2 = KeyBindingHelper.registerKeyBinding(new KeyBinding(
            "key.mcgm.pos2",
            GLFW.GLFW_KEY_RIGHT_BRACKET,
            "key.categories.mcgm"
        ));

        keyExportScreen = KeyBindingHelper.registerKeyBinding(new KeyBinding(
            "key.mcgm.export",
            GLFW.GLFW_KEY_M,
            "key.categories.mcgm"
        ));

        // 2. Register Client Tick Event for Keybindings and Deferred Screen Opening
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (pendingScreen != null) {
                net.minecraft.client.gui.screen.Screen s = pendingScreen;
                pendingScreen = null;
                try {
                    client.setScreen(s);
                } catch (Throwable t) {
                    t.printStackTrace();
                    if (client.player != null) {
                        client.player.sendMessage(Text.literal("§c[MCGM] Failed to open export screen: " + t.getMessage()), false);
                    }
                }
            }

            while (keyPos1.wasPressed()) {
                SelectionManager.getInstance().setPos1FromPlayer(client);
            }
            while (keyPos2.wasPressed()) {
                SelectionManager.getInstance().setPos2FromPlayer(client);
            }
            while (keyExportScreen.wasPressed()) {
                openScreenNextTick(new McgmExportScreen(client.currentScreen));
            }
        });

        // 3. Register Client Commands (/mcgm, /mcgm gui, /mcgm pos1, /mcgm pos2, /mcgm clear)
        ClientCommandRegistrationCallback.EVENT.register((dispatcher, registryAccess) -> {
            dispatcher.register(ClientCommandManager.literal("mcgm")
                .executes(context -> {
                    context.getSource().sendFeedback(Text.literal("§6[MCGM]§a Opening export menu..."));
                    openScreenNextTick(new McgmExportScreen(null));
                    return 1;
                })
                .then(ClientCommandManager.literal("menu")
                    .executes(context -> {
                        context.getSource().sendFeedback(Text.literal("§6[MCGM]§a Opening export menu..."));
                        openScreenNextTick(new McgmExportScreen(null));
                        return 1;
                    })
                )
                .then(ClientCommandManager.literal("export")
                    .executes(context -> {
                        context.getSource().sendFeedback(Text.literal("§6[MCGM]§a Opening export menu..."));
                        openScreenNextTick(new McgmExportScreen(null));
                        return 1;
                    })
                )
                .then(ClientCommandManager.literal("gui")
                    .executes(context -> {
                        MinecraftClient client = context.getSource().getClient();
                        SelectionManager sm = SelectionManager.getInstance();
                        BlockPos min = sm.getMinPos();
                        BlockPos max = sm.getMaxPos();
                        if (sm.getPos1() != null && sm.getPos2() != null && min != null && max != null) {
                            context.getSource().sendFeedback(Text.literal("§6[MCGM]§a Launching MCGM GUI program with current coordinates..."));
                            Exporter.launchGui(
                                client,
                                min.getX(), min.getY(), min.getZ(),
                                max.getX(), max.getY(), max.getZ(),
                                McgmExportScreen.getLastMapName(),
                                McgmExportScreen.getLastCategory()
                            );
                        } else {
                            context.getSource().sendError(Text.literal("§c[MCGM] No selection active! Select Pos 1 and Pos 2 first."));
                        }
                        return 1;
                    })
                )
                .then(ClientCommandManager.literal("pos1")
                    .executes(context -> {
                        MinecraftClient client = context.getSource().getClient();
                        SelectionManager.getInstance().setPos1FromPlayer(client);
                        return 1;
                    })
                    .then(ClientCommandManager.argument("pos", ClientPosArgumentType.pos())
                        .executes(context -> {
                            FabricClientCommandSource source = context.getSource();
                            BlockPos pos = ClientPosArgumentType.getBlockPos(context, "pos");
                            SelectionManager.getInstance().setPos1FromCommand(source.getClient(), pos);
                            return 1;
                        })
                    )
                )
                .then(ClientCommandManager.literal("pos2")
                    .executes(context -> {
                        MinecraftClient client = context.getSource().getClient();
                        SelectionManager.getInstance().setPos2FromPlayer(client);
                        return 1;
                    })
                    .then(ClientCommandManager.argument("pos", ClientPosArgumentType.pos())
                        .executes(context -> {
                            FabricClientCommandSource source = context.getSource();
                            BlockPos pos = ClientPosArgumentType.getBlockPos(context, "pos");
                            SelectionManager.getInstance().setPos2FromCommand(source.getClient(), pos);
                            return 1;
                        })
                    )
                )
                .then(ClientCommandManager.literal("clear")
                    .executes(context -> {
                        SelectionManager.getInstance().clear();
                        context.getSource().sendFeedback(Text.literal("§6[MCGM]§r Selection cleared."));
                        return 1;
                    })
                )
            );
        });

        // 4. Register 3D Wireframe Bounding Box World Renderer
        BoundingBoxRenderer.register();
    }

    /**
     * Client-side BlockPos argument supporting absolute numbers and relative (~ / ~offset) coordinates.
     */
    private static final class ClientPosArgumentType implements ArgumentType<ClientPosArgumentType.ClientPos> {
        private static final Collection<String> EXAMPLES = Arrays.asList("0 0 0", "~ ~ ~", "~5 ~-2 ~10");

        static ClientPosArgumentType pos() {
            return new ClientPosArgumentType();
        }

        static BlockPos getBlockPos(CommandContext<FabricClientCommandSource> context, String name) {
            ClientPos parsed = context.getArgument(name, ClientPos.class);
            Vec3d base = context.getSource().getPosition();
            return parsed.toBlockPos(base);
        }

        @Override
        public ClientPos parse(StringReader reader) throws CommandSyntaxException {
            int start = reader.getCursor();
            CoordinateArgument x = CoordinateArgument.parse(reader, false);
            if (!reader.canRead() || reader.peek() != ' ') {
                reader.setCursor(start);
                throw Vec3ArgumentType.INCOMPLETE_EXCEPTION.createWithContext(reader);
            }
            reader.skip();
            CoordinateArgument y = CoordinateArgument.parse(reader, false);
            if (!reader.canRead() || reader.peek() != ' ') {
                reader.setCursor(start);
                throw Vec3ArgumentType.INCOMPLETE_EXCEPTION.createWithContext(reader);
            }
            reader.skip();
            CoordinateArgument z = CoordinateArgument.parse(reader, false);
            return new ClientPos(x, y, z);
        }

        @Override
        public <S> CompletableFuture<Suggestions> listSuggestions(CommandContext<S> context, SuggestionsBuilder builder) {
            return BlockPosArgumentType.blockPos().listSuggestions(context, builder);
        }

        @Override
        public Collection<String> getExamples() {
            return EXAMPLES;
        }

        private record ClientPos(CoordinateArgument x, CoordinateArgument y, CoordinateArgument z) {
            BlockPos toBlockPos(Vec3d base) {
                return BlockPos.ofFloored(
                    x.toAbsoluteCoordinate(base.x),
                    y.toAbsoluteCoordinate(base.y),
                    z.toAbsoluteCoordinate(base.z)
                );
            }
        }
    }
}
