package com.mcgm.bridge;

import net.minecraft.client.gui.DrawContext;
import net.minecraft.client.gui.screen.Screen;
import net.minecraft.client.gui.widget.ButtonWidget;
import net.minecraft.client.gui.widget.CheckboxWidget;
import net.minecraft.client.gui.widget.TextFieldWidget;
import net.minecraft.text.Text;
import net.minecraft.util.math.BlockPos;

/**
 * Screen for configuring bounding box coordinates, pipeline flags, and launching the MCGM export pipeline.
 */
public class McgmExportScreen extends Screen {

    private static boolean settingsLoaded = false;
    private static String lastMapName = "gm_minecraft_world";
    private static String lastCategory = "mcgm";
    private static String languageMode = "auto"; // "auto", "ru", "en"
    private static String lightingStyle = "minecraft"; // "minecraft", "gmod", "bright", "night"
    private static boolean mcPhysics = true;
    private static boolean hqLight = false;
    private static boolean importGMod = true;
    private static boolean exportVmf = false;
    private static String dimensionMode = "auto"; // "auto", "overworld", "nether", "end"
    private static String worldOverride = "";

    private static void ensureSettingsLoadedStatic(net.minecraft.client.MinecraftClient client) {
        if (!settingsLoaded && client != null) {
            Exporter.ScreenSettings loaded = Exporter.loadScreenSettings(client);
            lastMapName = loaded.mapName;
            lastCategory = loaded.category;
            languageMode = loaded.languageMode;
            lightingStyle = loaded.lightingStyle;
            mcPhysics = loaded.mcPhysics;
            hqLight = loaded.hqLight;
            importGMod = loaded.importGMod;
            exportVmf = loaded.exportVmf;
            dimensionMode = loaded.dimensionMode;
            worldOverride = loaded.worldOverride;
            settingsLoaded = true;
        }
    }

    private void ensureSettingsLoaded() {
        ensureSettingsLoadedStatic(this.client);
    }

    private void persistSettings() {
        if (this.client == null) return;
        Exporter.ScreenSettings s = new Exporter.ScreenSettings();
        s.mapName = getLastMapName();
        s.category = getLastCategory();
        s.languageMode = languageMode;
        s.lightingStyle = lightingStyle;
        s.mcPhysics = mcPhysics;
        s.hqLight = hqLight;
        s.importGMod = importGMod;
        s.exportVmf = exportVmf;
        s.dimensionMode = dimensionMode;
        s.worldOverride = worldOverride != null ? worldOverride : "";
        Exporter.saveScreenSettings(this.client, s);
    }

    public String getEffectiveLanguage() {
        if ("auto".equalsIgnoreCase(languageMode)) {
            return Exporter.detectLanguage(this.client);
        }
        return languageMode;
    }

    private String getLanguageButtonText() {
        if ("auto".equalsIgnoreCase(languageMode)) {
            String detected = Exporter.detectLanguage(this.client).toUpperCase();
            return "Lang: Auto (" + detected + ")";
        }
        return "Lang: " + languageMode.toUpperCase();
    }

    private String getLightingButtonText(boolean isRu) {
        String label = switch (Exporter.sanitizeLightingStyle(lightingStyle)) {
            case "gmod" -> "GMod";
            case "bright" -> isRu ? "Яркий" : "Bright";
            case "night" -> isRu ? "Ночь" : "Night";
            default -> "Minecraft";
        };
        return (isRu ? "Свет: " : "Light: ") + label;
    }

    private String formatDimensionName(String dim, boolean isRu) {
        if (isRu) {
            return switch (dim) {
                case "nether" -> "Незер";
                case "end" -> "Энд";
                default -> "Верхний";
            };
        }
        return switch (dim) {
            case "nether" -> "Nether";
            case "end" -> "End";
            default -> "Overworld";
        };
    }

    private String getDimensionButtonText(boolean isRu) {
        String clean = Exporter.sanitizeDimensionMode(dimensionMode);
        if ("auto".equals(clean)) {
            String detected = Exporter.detectDimension(this.client);
            return (isRu ? "Мир: Авто (" : "Dim: Auto (") + formatDimensionName(detected, isRu) + ")";
        }
        return (isRu ? "Мир: " : "Dim: ") + formatDimensionName(clean, isRu);
    }

    public static String getLastMapName() {
        ensureSettingsLoadedStatic(net.minecraft.client.MinecraftClient.getInstance());
        return lastMapName != null && !lastMapName.isEmpty() ? lastMapName : "gm_minecraft_world";
    }

    public static void setLastMapName(String mapName) {
        if (mapName != null && !mapName.trim().isEmpty()) {
            lastMapName = Exporter.sanitizeMapName(mapName);
        }
    }

    public static String getLastCategory() {
        ensureSettingsLoadedStatic(net.minecraft.client.MinecraftClient.getInstance());
        return lastCategory != null && !lastCategory.isEmpty() ? lastCategory : "mcgm";
    }

    public static void setLastCategory(String category) {
        if (category != null && !category.trim().isEmpty()) {
            lastCategory = Exporter.sanitizeCategory(category);
        }
    }

    private final Screen parent;

    private TextFieldWidget fieldMinX;
    private TextFieldWidget fieldMinY;
    private TextFieldWidget fieldMinZ;
    private TextFieldWidget fieldMaxX;
    private TextFieldWidget fieldMaxY;
    private TextFieldWidget fieldMaxZ;

    private TextFieldWidget fieldMapName;
    private TextFieldWidget fieldCategory;

    private TextFieldWidget fieldMcgmPath;
    private TextFieldWidget fieldWorldOverride;

    private ButtonWidget btnLang;
    private ButtonWidget btnTakePos;
    private ButtonWidget btnRadius32;
    private ButtonWidget btnLightingStyle;
    private ButtonWidget btnDimension;
    private ButtonWidget btnExport;
    private ButtonWidget btnOpenGui;
    private ButtonWidget btnClose;

    private CheckboxWidget cbMcPhysics;
    private CheckboxWidget cbHqLight;
    private CheckboxWidget cbImportGMod;
    private CheckboxWidget cbExportVmf;

    private String statusError = "";
    private String mcgmPathDraft = null;
    private int topOffset = 0;

    public McgmExportScreen(Screen parent) {
        super(Text.translatable("screen.mcgm.title"));
        this.parent = parent;
    }

    private void captureWidgetState() {
        if (fieldMapName != null) {
            String txt = fieldMapName.getText().trim();
            if (!txt.isEmpty()) {
                lastMapName = Exporter.sanitizeMapName(txt);
            }
        }
        if (fieldCategory != null) {
            String txt = fieldCategory.getText().trim();
            if (!txt.isEmpty()) {
                lastCategory = Exporter.sanitizeCategory(txt);
            }
        }
        if (fieldWorldOverride != null) {
            worldOverride = fieldWorldOverride.getText().trim();
        }
        if (fieldMcgmPath != null) {
            mcgmPathDraft = fieldMcgmPath.getText();
        }
        if (cbMcPhysics != null) mcPhysics = cbMcPhysics.isChecked();
        if (cbHqLight != null) hqLight = cbHqLight.isChecked();
        if (cbImportGMod != null) importGMod = cbImportGMod.isChecked();
        if (cbExportVmf != null) exportVmf = cbExportVmf.isChecked();
    }

    @Override
    protected void init() {
        ensureSettingsLoaded();
        if (fieldMapName != null) {
            captureWidgetState();
            syncSelection();
        }
        super.init();

        this.topOffset = (this.height > 256) ? Math.min(20, (this.height - 240) / 6) : 0;
        int centerX = this.width / 2;
        boolean isRu = "ru".equals(getEffectiveLanguage());

        // Header: Language toggle button
        btnLang = ButtonWidget.builder(
            Text.literal(getLanguageButtonText()),
            b -> {
                if ("auto".equalsIgnoreCase(languageMode)) {
                    languageMode = "ru";
                } else if ("ru".equalsIgnoreCase(languageMode)) {
                    languageMode = "en";
                } else {
                    languageMode = "auto";
                }
                captureWidgetState();
                persistSettings();
                refreshLocalizedLabels();
            }
        ).dimensions(centerX + 95, topOffset + 3, 90, 16).build();
        this.addDrawableChild(btnLang);

        SelectionManager sm = SelectionManager.getInstance();
        BlockPos min = sm.getMinPos();
        BlockPos max = sm.getMaxPos();

        if (min == null || max == null) {
            if (this.client != null && this.client.player != null) {
                BlockPos p = this.client.player.getBlockPos();
                min = p;
                max = p;
            } else {
                min = BlockPos.ORIGIN;
                max = BlockPos.ORIGIN;
            }
        }

        // Row 1: Min coordinates (X, Y, Z) and "Take my pos" button
        int y1 = topOffset + 23;
        fieldMinX = new TextFieldWidget(this.textRenderer, centerX - 120, y1, 46, 18, Text.literal("Min X"));
        fieldMinX.setText(String.valueOf(min.getX()));
        this.addDrawableChild(fieldMinX);

        fieldMinY = new TextFieldWidget(this.textRenderer, centerX - 71, y1, 46, 18, Text.literal("Min Y"));
        fieldMinY.setText(String.valueOf(min.getY()));
        this.addDrawableChild(fieldMinY);

        fieldMinZ = new TextFieldWidget(this.textRenderer, centerX - 22, y1, 46, 18, Text.literal("Min Z"));
        fieldMinZ.setText(String.valueOf(min.getZ()));
        this.addDrawableChild(fieldMinZ);

        btnTakePos = ButtonWidget.builder(
            Text.literal(isRu ? "Моя позиция" : "Take my pos"),
            b -> onTakeMyPos()
        ).dimensions(centerX + 30, y1, 155, 18).build();
        this.addDrawableChild(btnTakePos);

        // Row 2: Max coordinates (X, Y, Z) and "Radius 32 around me" button
        int y2 = topOffset + 43;
        fieldMaxX = new TextFieldWidget(this.textRenderer, centerX - 120, y2, 46, 18, Text.literal("Max X"));
        fieldMaxX.setText(String.valueOf(max.getX()));
        this.addDrawableChild(fieldMaxX);

        fieldMaxY = new TextFieldWidget(this.textRenderer, centerX - 71, y2, 46, 18, Text.literal("Max Y"));
        fieldMaxY.setText(String.valueOf(max.getY()));
        this.addDrawableChild(fieldMaxY);

        fieldMaxZ = new TextFieldWidget(this.textRenderer, centerX - 22, y2, 46, 18, Text.literal("Max Z"));
        fieldMaxZ.setText(String.valueOf(max.getZ()));
        this.addDrawableChild(fieldMaxZ);

        btnRadius32 = ButtonWidget.builder(
            Text.literal(isRu ? "Радиус 32 вокруг меня" : "Radius 32 around me"),
            b -> onRadius32()
        ).dimensions(centerX + 30, y2, 155, 18).build();
        this.addDrawableChild(btnRadius32);

        // Row 3: Map name and Category fields
        int y3 = topOffset + 74;
        fieldMapName = new TextFieldWidget(this.textRenderer, centerX - 190, y3, 185, 18, Text.literal("Map Name"));
        fieldMapName.setMaxLength(64);
        fieldMapName.setText(getLastMapName());
        this.addDrawableChild(fieldMapName);

        fieldCategory = new TextFieldWidget(this.textRenderer, centerX + 5, y3, 180, 18, Text.literal("Category"));
        fieldCategory.setMaxLength(64);
        fieldCategory.setText(getLastCategory());
        this.addDrawableChild(fieldCategory);

        // Row 4: Lighting preset & Dimension selector
        int y4 = topOffset + 95;
        btnLightingStyle = ButtonWidget.builder(
            Text.literal(getLightingButtonText(isRu)),
            b -> {
                lightingStyle = switch (Exporter.sanitizeLightingStyle(lightingStyle)) {
                    case "minecraft" -> "gmod";
                    case "gmod" -> "bright";
                    case "bright" -> "night";
                    default -> "minecraft";
                };
                persistSettings();
                refreshLocalizedLabels();
            }
        ).dimensions(centerX - 190, y4, 185, 18).build();
        this.addDrawableChild(btnLightingStyle);

        btnDimension = ButtonWidget.builder(
            Text.literal(getDimensionButtonText(isRu)),
            b -> {
                dimensionMode = switch (Exporter.sanitizeDimensionMode(dimensionMode)) {
                    case "auto" -> "overworld";
                    case "overworld" -> "nether";
                    case "nether" -> "end";
                    default -> "auto";
                };
                persistSettings();
                refreshLocalizedLabels();
            }
        ).dimensions(centerX + 5, y4, 180, 18).build();
        this.addDrawableChild(btnDimension);

        // Row 5: Checkboxes row 1 (wide 185/180px so labels never clip)
        int y5 = topOffset + 116;
        cbMcPhysics = new CheckboxWidget(
            centerX - 190, y5, 185, 20,
            Text.literal(isRu ? "Физика Minecraft" : "MC Physics"),
            mcPhysics
        );
        this.addDrawableChild(cbMcPhysics);

        cbHqLight = new CheckboxWidget(
            centerX + 5, y5, 180, 20,
            Text.literal(isRu ? "HQ свет (медленно)" : "HQ Lighting (Slow)"),
            hqLight
        );
        this.addDrawableChild(cbHqLight);

        // Row 6: Checkboxes row 2
        int y6 = topOffset + 138;
        cbImportGMod = new CheckboxWidget(
            centerX - 190, y6, 185, 20,
            Text.literal(isRu ? "Импорт в GMod" : "Import to GMod"),
            importGMod
        );
        this.addDrawableChild(cbImportGMod);

        cbExportVmf = new CheckboxWidget(
            centerX + 5, y6, 180, 20,
            Text.literal(isRu ? "Экспорт VMF" : "Export Hammer VMF"),
            exportVmf
        );
        this.addDrawableChild(cbExportVmf);

        // Row 7: mcgm.exe Path & World Override (for multiplayer / custom local save)
        int y7 = topOffset + 171;
        fieldMcgmPath = new TextFieldWidget(this.textRenderer, centerX - 190, y7, 185, 18, Text.literal("mcgm.exe Path"));
        fieldMcgmPath.setMaxLength(512);
        fieldMcgmPath.setText(mcgmPathDraft != null ? mcgmPathDraft : Exporter.getOrResolveMcgmPath(this.client));
        fieldMcgmPath.setChangedListener(val -> statusError = "");
        this.addDrawableChild(fieldMcgmPath);

        fieldWorldOverride = new TextFieldWidget(this.textRenderer, centerX + 5, y7, 180, 18, Text.literal("World Override"));
        fieldWorldOverride.setMaxLength(512);
        fieldWorldOverride.setText(worldOverride != null ? worldOverride : "");
        fieldWorldOverride.setPlaceholder(Text.literal(isRu ? "Авто (одиночный мир)" : "Auto (Active SP)"));
        fieldWorldOverride.setChangedListener(val -> statusError = "");
        this.addDrawableChild(fieldWorldOverride);

        // Row 9: Action buttons
        int y9 = topOffset + 204;
        btnExport = ButtonWidget.builder(
            Text.literal(isRu ? "Экспорт в GMod" : "Export to GMod"),
            b -> onExportClicked()
        ).dimensions(centerX - 190, y9, 140, 20).build();
        this.addDrawableChild(btnExport);

        btnOpenGui = ButtonWidget.builder(
            Text.literal(isRu ? "Открыть MCGM GUI" : "Open MCGM GUI"),
            b -> onOpenGuiClicked()
        ).dimensions(centerX - 45, y9, 140, 20).build();
        this.addDrawableChild(btnOpenGui);

        btnClose = ButtonWidget.builder(
            Text.literal(isRu ? "Закрыть" : "Close"),
            b -> this.close()
        ).dimensions(centerX + 100, y9, 85, 20).build();
        this.addDrawableChild(btnClose);
    }

    private void refreshLocalizedLabels() {
        boolean isRu = "ru".equals(getEffectiveLanguage());
        if (btnLang != null) btnLang.setMessage(Text.literal(getLanguageButtonText()));
        if (btnTakePos != null) btnTakePos.setMessage(Text.literal(isRu ? "Моя позиция" : "Take my pos"));
        if (btnRadius32 != null) btnRadius32.setMessage(Text.literal(isRu ? "Радиус 32 вокруг меня" : "Radius 32 around me"));
        if (btnLightingStyle != null) btnLightingStyle.setMessage(Text.literal(getLightingButtonText(isRu)));
        if (btnDimension != null) btnDimension.setMessage(Text.literal(getDimensionButtonText(isRu)));
        if (cbMcPhysics != null) cbMcPhysics.setMessage(Text.literal(isRu ? "Физика Minecraft" : "MC Physics"));
        if (cbHqLight != null) cbHqLight.setMessage(Text.literal(isRu ? "HQ свет (медленно)" : "HQ Lighting (Slow)"));
        if (cbImportGMod != null) cbImportGMod.setMessage(Text.literal(isRu ? "Импорт в GMod" : "Import to GMod"));
        if (cbExportVmf != null) cbExportVmf.setMessage(Text.literal(isRu ? "Экспорт VMF" : "Export Hammer VMF"));
        if (fieldWorldOverride != null) {
            fieldWorldOverride.setPlaceholder(Text.literal(isRu ? "Авто (одиночный мир)" : "Auto (Active SP)"));
        }
        if (btnExport != null) btnExport.setMessage(Text.literal(isRu ? "Экспорт в GMod" : "Export to GMod"));
        if (btnOpenGui != null) btnOpenGui.setMessage(Text.literal(isRu ? "Открыть MCGM GUI" : "Open MCGM GUI"));
        if (btnClose != null) btnClose.setMessage(Text.literal(isRu ? "Закрыть" : "Close"));
    }

    private void onTakeMyPos() {
        statusError = "";
        if (this.client != null && this.client.player != null) {
            BlockPos p = this.client.player.getBlockPos();
            fieldMinX.setText(String.valueOf(p.getX()));
            fieldMinY.setText(String.valueOf(p.getY()));
            fieldMinZ.setText(String.valueOf(p.getZ()));
            fieldMaxX.setText(String.valueOf(p.getX()));
            fieldMaxY.setText(String.valueOf(p.getY()));
            fieldMaxZ.setText(String.valueOf(p.getZ()));
            syncSelection();
        }
    }

    private void onRadius32() {
        statusError = "";
        if (this.client != null && this.client.player != null) {
            BlockPos p = this.client.player.getBlockPos();
            int r = 32;
            int minYBound = (this.client.world != null) ? this.client.world.getBottomY() : -64;
            int maxYBound = (this.client.world != null) ? (this.client.world.getTopY() - 1) : 319;
            int minY = Math.max(minYBound, p.getY() - r);
            int maxY = Math.min(maxYBound, p.getY() + r);
            if (minY > maxY) {
                int clamped = Math.max(minYBound, Math.min(maxYBound, p.getY()));
                minY = clamped;
                maxY = clamped;
            }
            fieldMinX.setText(String.valueOf(p.getX() - r));
            fieldMinY.setText(String.valueOf(minY));
            fieldMinZ.setText(String.valueOf(p.getZ() - r));
            fieldMaxX.setText(String.valueOf(p.getX() + r));
            fieldMaxY.setText(String.valueOf(maxY));
            fieldMaxZ.setText(String.valueOf(p.getZ() + r));
            syncSelection();
        }
    }

    private int parseCoord(String text, int fallback) {
        try {
            return Integer.parseInt(text.trim());
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    private Integer parseCoordStrict(String text) {
        if (text == null) return null;
        try {
            return Integer.parseInt(text.trim());
        } catch (NumberFormatException e) {
            return null;
        }
    }

    private void syncSelection() {
        if (fieldMinX == null || fieldMaxX == null) return;
        int x1 = parseCoord(fieldMinX.getText(), 0);
        int y1 = parseCoord(fieldMinY.getText(), 0);
        int z1 = parseCoord(fieldMinZ.getText(), 0);
        int x2 = parseCoord(fieldMaxX.getText(), 0);
        int y2 = parseCoord(fieldMaxY.getText(), 0);
        int z2 = parseCoord(fieldMaxZ.getText(), 0);
        SelectionManager.getInstance().setCoords(x1, y1, z1, x2, y2, z2);
    }

    private void onExportClicked() {
        statusError = "";
        Integer px1 = parseCoordStrict(fieldMinX.getText());
        Integer py1 = parseCoordStrict(fieldMinY.getText());
        Integer pz1 = parseCoordStrict(fieldMinZ.getText());
        Integer px2 = parseCoordStrict(fieldMaxX.getText());
        Integer py2 = parseCoordStrict(fieldMaxY.getText());
        Integer pz2 = parseCoordStrict(fieldMaxZ.getText());
        if (px1 == null || py1 == null || pz1 == null || px2 == null || py2 == null || pz2 == null) {
            boolean isRu = "ru".equals(getEffectiveLanguage());
            statusError = isRu ? "Ошибка: введите целые числа во все координаты" : "Error: all coordinates must be integers";
            return;
        }

        syncSelection();
        captureWidgetState();
        persistSettings();

        int x1 = px1;
        int y1 = py1;
        int z1 = pz1;
        int x2 = px2;
        int y2 = py2;
        int z2 = pz2;

        int minX = Math.min(x1, x2);
        int minY = Math.min(y1, y2);
        int minZ = Math.min(z1, z2);
        int maxX = Math.max(x1, x2);
        int maxY = Math.max(y1, y2);
        int maxZ = Math.max(z1, z2);

        String mapName = getLastMapName();
        String category = getLastCategory();
        String customPath = (fieldMcgmPath != null) ? fieldMcgmPath.getText().trim() : null;

        boolean launched = Exporter.launchExport(
            this.client,
            minX, minY, minZ,
            maxX, maxY, maxZ,
            mapName,
            category,
            lightingStyle,
            mcPhysics,
            hqLight,
            importGMod,
            exportVmf,
            dimensionMode,
            worldOverride,
            customPath,
            languageMode
        );
        if (launched) {
            this.close();
        } else {
            statusError = Exporter.getLastLaunchError();
        }
    }

    private void onOpenGuiClicked() {
        statusError = "";
        Integer px1 = parseCoordStrict(fieldMinX.getText());
        Integer py1 = parseCoordStrict(fieldMinY.getText());
        Integer pz1 = parseCoordStrict(fieldMinZ.getText());
        Integer px2 = parseCoordStrict(fieldMaxX.getText());
        Integer py2 = parseCoordStrict(fieldMaxY.getText());
        Integer pz2 = parseCoordStrict(fieldMaxZ.getText());
        if (px1 == null || py1 == null || pz1 == null || px2 == null || py2 == null || pz2 == null) {
            boolean isRu = "ru".equals(getEffectiveLanguage());
            statusError = isRu ? "Ошибка: введите целые числа во все координаты" : "Error: all coordinates must be integers";
            return;
        }

        syncSelection();
        captureWidgetState();
        persistSettings();

        int x1 = px1;
        int y1 = py1;
        int z1 = pz1;
        int x2 = px2;
        int y2 = py2;
        int z2 = pz2;

        int minX = Math.min(x1, x2);
        int minY = Math.min(y1, y2);
        int minZ = Math.min(z1, z2);
        int maxX = Math.max(x1, x2);
        int maxY = Math.max(y1, y2);
        int maxZ = Math.max(z1, z2);

        String mapName = getLastMapName();
        String category = getLastCategory();
        String customPath = (fieldMcgmPath != null) ? fieldMcgmPath.getText().trim() : null;

        boolean launched = Exporter.launchGui(
            this.client,
            minX, minY, minZ,
            maxX, maxY, maxZ,
            mapName,
            category,
            lightingStyle,
            mcPhysics,
            hqLight,
            dimensionMode,
            worldOverride,
            customPath,
            languageMode
        );
        if (launched) {
            this.close();
        } else {
            statusError = Exporter.getLastLaunchError();
        }
    }

    @Override
    public void render(DrawContext context, int mouseX, int mouseY, float delta) {
        this.renderBackground(context);

        int centerX = this.width / 2;
        boolean isRu = "ru".equals(getEffectiveLanguage());

        // Title
        context.drawCenteredTextWithShadow(this.textRenderer, Text.literal("MCGM Settings"), centerX, topOffset + 7, 0xFFFFFF);

        // Coordinate row labels
        String lblMin = isRu ? "Мин X, Y, Z:" : "Min X, Y, Z:";
        String lblMax = isRu ? "Макс X, Y, Z:" : "Max X, Y, Z:";
        context.drawTextWithShadow(this.textRenderer, Text.literal(lblMin), centerX - 190, topOffset + 28, 0xCCCCCC);
        context.drawTextWithShadow(this.textRenderer, Text.literal(lblMax), centerX - 190, topOffset + 48, 0xCCCCCC);

        // Map & Category labels
        context.drawTextWithShadow(
            this.textRenderer,
            Text.literal(isRu ? "Имя карты:" : "Map Name:"),
            centerX - 190, topOffset + 64, 0xCCCCCC
        );
        context.drawTextWithShadow(
            this.textRenderer,
            Text.literal(isRu ? "Категория:" : "Category:"),
            centerX + 5, topOffset + 64, 0xCCCCCC
        );

        // Path & World Override labels
        context.drawTextWithShadow(
            this.textRenderer,
            Text.literal(isRu ? "Путь к mcgm.exe:" : "Path to mcgm.exe:"),
            centerX - 190, topOffset + 161, 0xCCCCCC
        );

        boolean remoteMp = Exporter.isRemoteMultiplayer(this.client);
        boolean overrideEmpty = fieldWorldOverride == null || fieldWorldOverride.getText().trim().isEmpty();
        if (remoteMp && overrideEmpty) {
            context.drawTextWithShadow(
                this.textRenderer,
                Text.literal(isRu ? "MP: Укажите путь к миру!" : "MP: Local world path required!"),
                centerX + 5, topOffset + 161, 0xFF6666
            );
        } else {
            context.drawTextWithShadow(
                this.textRenderer,
                Text.literal(isRu ? "Путь к миру (MP):" : "World Override (MP):"),
                centerX + 5, topOffset + 161, 0xCCCCCC
            );
        }

        // Dynamic volume / error / warning line
        if (statusError != null && !statusError.isEmpty()) {
            String clippedErr = this.textRenderer.trimToWidth(statusError, 375);
            context.drawCenteredTextWithShadow(this.textRenderer, Text.literal(clippedErr), centerX, topOffset + 192, 0xFF5555);
        } else {
            int x1 = parseCoord(fieldMinX.getText(), 0);
            int y1 = parseCoord(fieldMinY.getText(), 0);
            int z1 = parseCoord(fieldMinZ.getText(), 0);
            int x2 = parseCoord(fieldMaxX.getText(), 0);
            int y2 = parseCoord(fieldMaxY.getText(), 0);
            int z2 = parseCoord(fieldMaxZ.getText(), 0);

            long dx = Math.abs((long) x2 - (long) x1) + 1L;
            long dy = Math.abs((long) y2 - (long) y1) + 1L;
            long dz = Math.abs((long) z2 - (long) z1) + 1L;
            long total = dx * dy * dz;

            int color = (total > 1_000_000L) ? 0xFFAA00 : 0x55FF55;
            String volStr = isRu
                ? String.format("Регион: %d × %d × %d (%s блоков)", dx, dy, dz, String.format("%,d", total))
                : String.format("Selection: %d × %d × %d (%s blocks)", dx, dy, dz, String.format("%,d", total));
            if (remoteMp && overrideEmpty) {
                volStr += isRu ? " [MP: укажите мир]" : " [MP: set world path]";
                color = 0xFFAA00;
            }
            context.drawCenteredTextWithShadow(this.textRenderer, Text.literal(volStr), centerX, topOffset + 192, color);
        }

        super.render(context, mouseX, mouseY, delta);
    }

    @Override
    public void close() {
        captureWidgetState();
        if (fieldMcgmPath != null && !fieldMcgmPath.getText().trim().isEmpty()) {
            Exporter.saveMcgmPath(this.client, fieldMcgmPath.getText().trim());
        }
        persistSettings();
        if (this.client != null) {
            this.client.setScreen(this.parent);
        }
    }
}
