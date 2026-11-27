package com.mcgm.bridge;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import net.minecraft.client.MinecraftClient;
import net.minecraft.registry.RegistryKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.text.Text;
import net.minecraft.util.WorldSavePath;
import net.minecraft.world.World;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.regex.Pattern;

/**
 * Handles world saving flushes, config persistence, and launching the mcgm CLI or GUI executable via ProcessBuilder.
 */
public class Exporter {

    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();
    private static final Pattern PAT_MAP_INVALID = Pattern.compile("[^a-zA-Z0-9_\\-]");
    private static final Pattern PAT_MULTI_UNDERSCORE = Pattern.compile("_+");
    private static final Pattern PAT_WIN_RESERVED = Pattern.compile("^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$");
    private static final Pattern PAT_CAT_INVALID = Pattern.compile("[^a-zA-Z0-9_\\-/]");
    private static final Pattern PAT_CUSTOM_DIM = Pattern.compile("^[a-z0-9_.-]+:[a-z0-9_./-]+$");
    private static final Pattern PAT_NOISY_SEP = Pattern.compile("^[=\\-_*]{4,}$");
    private static final Pattern PAT_PCT_START = Pattern.compile("^\\[\\d{1,3}%\\].*");
    private static final Pattern PAT_PCT_ANY = Pattern.compile(".*\\[\\d{1,3}%\\].*");
    private static String cachedMcgmPath = null;
    private static volatile String lastLaunchError = "";
    private static final AtomicBoolean EXPORT_RUNNING = new AtomicBoolean(false);

    public static class ScreenSettings {
        public String mapName = "gm_minecraft_world";
        public String category = "mcgm";
        public String languageMode = "auto";
        public String lightingStyle = "minecraft";
        public boolean mcPhysics = true;
        public boolean hqLight = false;
        public boolean importGMod = true;
        public boolean exportVmf = false;
        public String dimensionMode = "auto";
        public String worldOverride = "";
    }

    public static String getLastLaunchError() {
        return lastLaunchError != null ? lastLaunchError : "";
    }

    private static void setLastLaunchError(String err) {
        lastLaunchError = (err != null) ? err : "";
    }

    public static String sanitizeMapName(String input) {
        if (input == null) {
            return "gm_minecraft_world";
        }
        String name = input.trim();
        if (name.isEmpty()) {
            return "gm_minecraft_world";
        }
        if (name.toLowerCase(Locale.ROOT).endsWith(".bsp")) {
            name = name.substring(0, name.length() - 4).trim();
        } else if (name.toLowerCase(Locale.ROOT).endsWith(".vmf")) {
            name = name.substring(0, name.length() - 4).trim();
        }
        name = PAT_MAP_INVALID.matcher(name).replaceAll("_");
        name = PAT_MULTI_UNDERSCORE.matcher(name).replaceAll("_");
        while (name.startsWith("_") || name.startsWith("-")) {
            name = name.substring(1);
        }
        while (name.endsWith("_") || name.endsWith("-")) {
            name = name.substring(0, name.length() - 1);
        }
        if (name.isEmpty()) {
            return "gm_minecraft_world";
        }
        String upper = name.toUpperCase(Locale.ROOT);
        if (PAT_WIN_RESERVED.matcher(upper).matches()) {
            name = "gm_" + name.toLowerCase(Locale.ROOT);
        }
        return name;
    }

    public static String sanitizeCategory(String input) {
        if (input == null) {
            return "mcgm";
        }
        String cat = input.trim().replace('\\', '/');
        while (cat.startsWith("/")) {
            cat = cat.substring(1);
        }
        while (cat.endsWith("/")) {
            cat = cat.substring(0, cat.length() - 1);
        }
        cat = PAT_CAT_INVALID.matcher(cat).replaceAll("_");
        cat = PAT_MULTI_UNDERSCORE.matcher(cat).replaceAll("_");
        if (cat.isEmpty()) {
            return "mcgm";
        }
        return cat;
    }

    public static String sanitizeLightingStyle(String style) {
        if (style == null) {
            return "minecraft";
        }
        String s = style.trim().toLowerCase(Locale.ROOT);
        return switch (s) {
            case "minecraft", "gmod", "bright", "night" -> s;
            default -> "minecraft";
        };
    }

    public static String sanitizeDimensionMode(String mode) {
        if (mode == null) {
            return "auto";
        }
        String m = mode.trim().toLowerCase(Locale.ROOT);
        return switch (m) {
            case "auto", "overworld", "nether", "end" -> m;
            case "minecraft:overworld" -> "overworld";
            case "minecraft:the_nether", "the_nether" -> "nether";
            case "minecraft:the_end", "the_end" -> "end";
            default -> {
                if (PAT_CUSTOM_DIM.matcher(m).matches() && !m.contains("..")) {
                    yield m;
                }
                yield "auto";
            }
        };
    }

    public static String sanitizeLanguageMode(String mode) {
        if (mode == null) {
            return "auto";
        }
        String m = mode.trim().toLowerCase(Locale.ROOT);
        return switch (m) {
            case "auto", "ru", "en" -> m;
            default -> "auto";
        };
    }

    public static File getConfigFile(MinecraftClient client) {
        File runDir = (client != null && client.runDirectory != null) ? client.runDirectory : new File(".");
        File configDir = new File(runDir, "config");
        if (!configDir.exists()) {
            configDir.mkdirs();
        }
        return new File(configDir, "mcgm.json");
    }

    private static JsonObject readConfigJson(MinecraftClient client) {
        File configFile = getConfigFile(client);
        if (!configFile.isFile()) {
            return new JsonObject();
        }
        try {
            String content = Files.readString(configFile.toPath(), StandardCharsets.UTF_8);
            if (content != null && !content.trim().isEmpty()) {
                JsonElement el = JsonParser.parseString(content);
                if (el != null && el.isJsonObject()) {
                    return el.getAsJsonObject();
                }
            }
        } catch (Exception ignored) {
        }
        return new JsonObject();
    }

    private static void writeConfigJson(MinecraftClient client, JsonObject obj) {
        File configFile = getConfigFile(client);
        try {
            Files.writeString(configFile.toPath(), GSON.toJson(obj) + "\n", StandardCharsets.UTF_8);
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    private static String getJsonString(JsonObject obj, String key, String fallback) {
        if (obj != null && obj.has(key) && obj.get(key).isJsonPrimitive()) {
            try {
                String v = obj.get(key).getAsString();
                return v != null ? v : fallback;
            } catch (Exception ignored) {
            }
        }
        return fallback;
    }

    private static boolean getJsonBoolean(JsonObject obj, String key, boolean fallback) {
        if (obj != null && obj.has(key) && obj.get(key).isJsonPrimitive()) {
            try {
                return obj.get(key).getAsBoolean();
            } catch (Exception ignored) {
            }
        }
        return fallback;
    }

    public static ScreenSettings loadScreenSettings(MinecraftClient client) {
        ScreenSettings s = new ScreenSettings();
        JsonObject obj = readConfigJson(client);
        s.mapName = sanitizeMapName(getJsonString(obj, "map_name", s.mapName));
        s.category = sanitizeCategory(getJsonString(obj, "category", s.category));
        s.languageMode = sanitizeLanguageMode(getJsonString(obj, "language_mode", s.languageMode));
        s.lightingStyle = sanitizeLightingStyle(getJsonString(obj, "lighting_style", s.lightingStyle));
        s.mcPhysics = getJsonBoolean(obj, "mc_physics", true);
        s.hqLight = getJsonBoolean(obj, "hq_light", false);
        s.importGMod = getJsonBoolean(obj, "import_gmod", true);
        s.exportVmf = getJsonBoolean(obj, "export_vmf", false);
        s.dimensionMode = sanitizeDimensionMode(getJsonString(obj, "dimension", s.dimensionMode));
        s.worldOverride = getJsonString(obj, "world_override", "").trim();
        return s;
    }

    public static void saveScreenSettings(MinecraftClient client, ScreenSettings s) {
        if (s == null) return;
        JsonObject obj = readConfigJson(client);
        obj.addProperty("map_name", sanitizeMapName(s.mapName));
        obj.addProperty("category", sanitizeCategory(s.category));
        obj.addProperty("language_mode", sanitizeLanguageMode(s.languageMode));
        obj.addProperty("lighting_style", sanitizeLightingStyle(s.lightingStyle));
        obj.addProperty("mc_physics", s.mcPhysics);
        obj.addProperty("hq_light", s.hqLight);
        obj.addProperty("import_gmod", s.importGMod);
        obj.addProperty("export_vmf", s.exportVmf);
        obj.addProperty("dimension", sanitizeDimensionMode(s.dimensionMode));
        obj.addProperty("world_override", s.worldOverride != null ? s.worldOverride.trim() : "");
        writeConfigJson(client, obj);
    }

    private static File resolveMcgmExecutable(File candidate) {
        if (candidate == null) {
            return null;
        }
        if (candidate.isFile() && candidate.getName().toLowerCase(Locale.ROOT).endsWith(".exe")) {
            return candidate;
        }
        if (candidate.isDirectory()) {
            File candExe = new File(candidate, "mcgm.exe");
            if (candExe.isFile()) {
                return candExe;
            }
        }
        return null;
    }

    private static boolean isValidMcgmTarget(File candidate) {
        return resolveMcgmExecutable(candidate) != null;
    }

    public static String getOrResolveMcgmPath(MinecraftClient client) {
        if (cachedMcgmPath != null && !cachedMcgmPath.isEmpty()) {
            File f = new File(cachedMcgmPath);
            if (isValidMcgmTarget(f)) {
                return cachedMcgmPath;
            }
        }

        // 1. Try reading from config/mcgm.json
        JsonObject cfg = readConfigJson(client);
        String cfgPath = getJsonString(cfg, "mcgm_path", "");
        if (!cfgPath.isEmpty()) {
            File f = new File(cfgPath);
            if (isValidMcgmTarget(f)) {
                cachedMcgmPath = f.getAbsolutePath();
                return cachedMcgmPath;
            }
        }

        // 2. Check environment variables
        String envPath = System.getenv("MCGM_PATH");
        if (envPath != null && !envPath.trim().isEmpty()) {
            File f = new File(envPath.trim());
            if (isValidMcgmTarget(f)) {
                cachedMcgmPath = f.getAbsolutePath();
                saveMcgmPath(client, cachedMcgmPath);
                return cachedMcgmPath;
            }
        }
        String envHome = System.getenv("MCGM_HOME");
        if (envHome != null && !envHome.trim().isEmpty()) {
            File candExe = new File(envHome.trim(), "mcgm.exe");
            if (candExe.isFile()) {
                cachedMcgmPath = candExe.getAbsolutePath();
                saveMcgmPath(client, cachedMcgmPath);
                return cachedMcgmPath;
            }
        }

        // 3. Search relative to Minecraft runDirectory
        File runDir = ((client != null && client.runDirectory != null) ? client.runDirectory : new File(".")).getAbsoluteFile();
        File parent1 = runDir.getParentFile();
        File parent2 = (parent1 != null) ? parent1.getParentFile() : null;
        File[] runCandidates = new File[] {
            new File(runDir, "mcgm/mcgm.exe"),
            new File(runDir, "mcgm.exe"),
            parent1 != null ? new File(parent1, "mcgm/mcgm.exe") : null,
            parent1 != null ? new File(parent1, "mcgm.exe") : null,
            parent2 != null ? new File(parent2, "mcgm/mcgm.exe") : null
        };
        for (File cand : runCandidates) {
            if (cand != null && cand.isFile()) {
                cachedMcgmPath = cand.getAbsolutePath();
                saveMcgmPath(client, cachedMcgmPath);
                return cachedMcgmPath;
            }
        }

        // 4. Standard dev & user paths
        String userHome = System.getProperty("user.home", "");
        File[] stdCandidates = new File[] {
            new File("mcgm.exe"),
            new File(userHome, "mcgm\\mcgm.exe"),
            new File(userHome, "AppData\\Local\\mcgm\\mcgm.exe"),
            new File("C:\\mcgm\\mcgm.exe"),
            new File("D:\\mcgm\\mcgm.exe")
        };
        for (File cand : stdCandidates) {
            if (cand.isFile()) {
                cachedMcgmPath = cand.getAbsolutePath();
                saveMcgmPath(client, cachedMcgmPath);
                return cachedMcgmPath;
            }
        }

        // 5. Search PATH
        String sysPath = System.getenv("PATH");
        if (sysPath != null) {
            for (String part : sysPath.split(File.pathSeparator)) {
                File cand = new File(part.trim(), "mcgm.exe");
                if (cand.isFile()) {
                    cachedMcgmPath = cand.getAbsolutePath();
                    saveMcgmPath(client, cachedMcgmPath);
                    return cachedMcgmPath;
                }
            }
        }

        return new File("mcgm.exe").getAbsolutePath();
    }

    public static void saveMcgmPath(MinecraftClient client, String path) {
        if (path == null || path.trim().isEmpty()) return;
        cachedMcgmPath = path.trim();
        JsonObject obj = readConfigJson(client);
        obj.addProperty("mcgm_path", cachedMcgmPath);
        writeConfigJson(client, obj);
    }

    public static boolean isRemoteMultiplayer(MinecraftClient client) {
        return client != null && client.world != null && client.getServer() == null;
    }

    public static boolean isValidWorldDirectory(File dir) {
        if (dir == null || !dir.isDirectory()) {
            return false;
        }
        return new File(dir, "level.dat").isFile()
            || new File(dir, "region").isDirectory()
            || new File(dir, "DIM-1/region").isDirectory()
            || new File(dir, "DIM1/region").isDirectory()
            || new File(dir, "dimensions").isDirectory();
    }

    /**
     * Detects the active local Singleplayer integrated server save directory.
     * Never silently falls back to an unrelated .minecraft/saves directory when on a remote MP/dedicated server.
     */
    public static String detectWorldSavePath(MinecraftClient client) {
        if (client != null) {
            MinecraftServer server = client.getServer();
            if (server != null) {
                try {
                    Path savePath = server.getSavePath(WorldSavePath.ROOT).toAbsolutePath().normalize();
                    if (Files.isDirectory(savePath)) {
                        return savePath.toString();
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                }
            }
        }
        return "";
    }

    public static String detectDimensionFromRegistryId(String registryId) {
        if (registryId == null) {
            return "overworld";
        }
        String norm = registryId.trim().toLowerCase(Locale.ROOT);
        if (norm.isEmpty() || "minecraft:overworld".equals(norm) || "overworld".equals(norm)) {
            return "overworld";
        }
        if ("minecraft:the_nether".equals(norm) || "nether".equals(norm) || "the_nether".equals(norm)) {
            return "nether";
        }
        if ("minecraft:the_end".equals(norm) || "end".equals(norm) || "the_end".equals(norm)) {
            return "end";
        }
        return norm;
    }

    public static String detectDimension(MinecraftClient client) {
        if (client != null && client.world != null) {
            RegistryKey<World> key = client.world.getRegistryKey();
            if (World.NETHER.equals(key)) {
                return "nether";
            } else if (World.END.equals(key)) {
                return "end";
            } else if (World.OVERWORLD.equals(key)) {
                return "overworld";
            }
            if (key != null && key.getValue() != null) {
                return detectDimensionFromRegistryId(key.getValue().toString());
            }
        }
        return "overworld";
    }

    public static String resolveDimension(MinecraftClient client, String dimensionMode) {
        String clean = sanitizeDimensionMode(dimensionMode);
        if ("auto".equals(clean)) {
            return detectDimension(client);
        }
        return clean;
    }

    public static void flushSingleplayerSave(MinecraftClient client) {
        if (client == null) return;
        MinecraftServer server = client.getServer();
        if (server != null) {
            try {
                // suppressLogs=false, flush=true, force=true
                if (server.isOnThread()) {
                    server.saveAll(false, true, true);
                } else {
                    server.submit(() -> server.saveAll(false, true, true)).join();
                }
            } catch (Exception e) {
                e.printStackTrace();
            }
        }
    }

    public static String detectLanguage(MinecraftClient client) {
        if (client != null && client.options != null && client.options.language != null) {
            String l = client.options.language.toLowerCase(Locale.ROOT);
            if (l.startsWith("ru")) return "ru";
        }
        if (client != null && client.getLanguageManager() != null && client.getLanguageManager().getLanguage() != null) {
            String l = client.getLanguageManager().getLanguage().toLowerCase(Locale.ROOT);
            if (l.startsWith("ru")) return "ru";
        }
        return "en";
    }

    private static String resolveLang(MinecraftClient client, String explicitLang) {
        if (explicitLang == null || explicitLang.trim().isEmpty() || "auto".equalsIgnoreCase(explicitLang)) {
            return detectLanguage(client);
        }
        return explicitLang.trim().toLowerCase(Locale.ROOT);
    }

    private static void sendChat(MinecraftClient client, String msg) {
        if (client != null && client.player != null && msg != null) {
            client.player.sendMessage(Text.literal(msg), false);
        }
    }

    private static String resolveWorldForLaunch(
        MinecraftClient client,
        String worldOverride,
        String lang,
        boolean requireWorld
    ) {
        boolean isRu = "ru".equals(lang);
        String trimmedOverride = (worldOverride != null) ? worldOverride.trim() : "";
        if (!trimmedOverride.isEmpty()) {
            File overrideDir = new File(trimmedOverride);
            if (!isValidWorldDirectory(overrideDir)) {
                String err = isRu
                    ? "Неверный путь к миру (нет level.dat или region): \"" + trimmedOverride + "\""
                    : "Invalid World Override path (missing level.dat or region): \"" + trimmedOverride + "\"";
                setLastLaunchError(err);
                sendChat(client, "§c§l[MCGM]§r §c" + err);
                return null;
            }
            if (isRemoteMultiplayer(client)) {
                String info = isRu
                    ? "§e[MCGM] Сетевая игра: используется указанная локальная копия мира §f\"" + overrideDir.getAbsolutePath() + "\"§e."
                    : "§e[MCGM] Multiplayer: using explicit local world override §f\"" + overrideDir.getAbsolutePath() + "\"§e.";
                sendChat(client, info);
            }
            return overrideDir.getAbsolutePath();
        }

        flushSingleplayerSave(client);
        String detected = detectWorldSavePath(client);
        if (!detected.isEmpty()) {
            return detected;
        }

        if (requireWorld) {
            boolean mp = isRemoteMultiplayer(client);
            String shortErr = mp
                ? (isRu
                    ? "Сетевая игра: укажите локальную копию мира в поле 'Путь к миру (MP)'"
                    : "Multiplayer detected: enter a local world copy in 'World Override'")
                : (isRu
                    ? "Локальный мир одиночной игры не найден. Откройте мир или укажите путь к миру."
                    : "No active local Singleplayer world detected. Join SP or set World Override.");
            setLastLaunchError(shortErr);
            String chatErr = mp
                ? (isRu
                    ? "§c§l[MCGM]§r §cОбнаружен сетевой / нелокальный сервер (client.getServer() == null). Автовыбор чужого сохранения из .minecraft/saves отключен!\n§eОткройте одиночную игру или укажите локальную копию мира в меню /mcgm (поле 'Путь к миру (MP)')."
                    : "§c§l[MCGM]§r §cMultiplayer / non-local server detected (client.getServer() == null). Refusing to guess an unrelated save in .minecraft/saves!\n§eJoin Singleplayer or specify an explicit local world folder in /mcgm ('World Override').")
                : ("§c§l[MCGM]§r §c" + shortErr);
            sendChat(client, chatErr);
            return null;
        }

        if (isRemoteMultiplayer(client)) {
            String warn = isRu
                ? "§e§l[MCGM]§r §eСетевая игра: путь к миру не передан в GUI (сервер удаленный). Выберите локальную копию мира в окне MCGM или укажите World Override."
                : "§e§l[MCGM]§r §eMultiplayer detected: no local --world path passed to MCGM GUI. Select a local world folder in MCGM GUI or set World Override.";
            sendChat(client, warn);
        }
        return "";
    }

    public static boolean launchExport(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        boolean lighting,
        boolean importGMod,
        boolean exportVmf
    ) {
        return launchExport(client, minX, minY, minZ, maxX, maxY, maxZ, mapName, category, lighting, importGMod, exportVmf, null, "auto");
    }

    public static boolean launchExport(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        boolean lighting,
        boolean importGMod,
        boolean exportVmf,
        String customMcgmPath
    ) {
        return launchExport(client, minX, minY, minZ, maxX, maxY, maxZ, mapName, category, lighting, importGMod, exportVmf, customMcgmPath, "auto");
    }

    public static boolean launchExport(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        boolean lighting,
        boolean importGMod,
        boolean exportVmf,
        String customMcgmPath,
        String explicitLang
    ) {
        ScreenSettings saved = loadScreenSettings(client);
        String style = lighting ? "minecraft" : "gmod";
        return launchExport(
            client,
            minX, minY, minZ,
            maxX, maxY, maxZ,
            mapName,
            category,
            style,
            saved.mcPhysics,
            saved.hqLight,
            importGMod,
            exportVmf,
            saved.dimensionMode,
            saved.worldOverride,
            customMcgmPath,
            explicitLang
        );
    }

    public static boolean launchExport(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        String lightingStyle,
        boolean mcPhysics,
        boolean hqLight,
        boolean importGMod,
        boolean exportVmf,
        String dimensionMode,
        String worldOverride,
        String customMcgmPath,
        String explicitLang
    ) {
        setLastLaunchError("");
        String resolvedPath;
        String trimmedCustom = customMcgmPath != null ? customMcgmPath.trim() : "";
        if (!trimmedCustom.isEmpty()) {
            File customExe = resolveMcgmExecutable(new File(trimmedCustom));
            if (customExe != null) {
                resolvedPath = customExe.getAbsolutePath();
                saveMcgmPath(client, resolvedPath);
            } else {
                resolvedPath = trimmedCustom;
            }
        } else {
            resolvedPath = getOrResolveMcgmPath(client);
        }
        File exeOrDir = new File(resolvedPath);
        File exeFile = resolveMcgmExecutable(exeOrDir);
        File workspaceDir = (exeFile != null) ? exeFile.getParentFile() : exeOrDir.getParentFile();

        if (workspaceDir == null || !workspaceDir.isDirectory()) {
            workspaceDir = (client != null && client.runDirectory != null) ? client.runDirectory : new File(".");
        }

        String cleanMap = sanitizeMapName(mapName);
        String cleanCategory = sanitizeCategory(category);
        String lang = resolveLang(client, explicitLang);
        String cleanStyle = sanitizeLightingStyle(lightingStyle);
        String resolvedDimension = resolveDimension(client, dimensionMode);

        List<String> cmd = new ArrayList<>();
        if (exeFile != null && exeFile.isFile()) {
            cmd.add(exeFile.getAbsolutePath());
        } else {
            boolean isRu = "ru".equals(lang);
            String shortErr = isRu
                ? "mcgm.exe не найден: \"" + resolvedPath + "\""
                : "mcgm.exe not found at: \"" + resolvedPath + "\"";
            setLastLaunchError(shortErr);
            String errMsg = isRu
                ? "§c§l[MCGM]§r §cОшибка: mcgm.exe не найден по пути: §f\"" + resolvedPath + "\"§c!" +
                    "\n§eУкажите правильный путь в поле 'Путь к mcgm.exe' в меню /mcgm или в config/mcgm.json"
                : "§c§l[MCGM]§r §cError: mcgm.exe not found at: §f\"" + resolvedPath + "\"§c!" +
                    "\n§eSet the valid path in the 'Path to mcgm.exe' field in /mcgm or in config/mcgm.json";
            sendChat(client, errMsg);
            return false;
        }

        String worldPath = resolveWorldForLaunch(client, worldOverride, lang, true);
        if (worldPath == null || worldPath.isEmpty()) {
            return false;
        }

        cmd.add("--world");
        cmd.add(worldPath);

        cmd.add("--coords");
        cmd.add(String.valueOf(minX));
        cmd.add(String.valueOf(minY));
        cmd.add(String.valueOf(minZ));
        cmd.add(String.valueOf(maxX));
        cmd.add(String.valueOf(maxY));
        cmd.add(String.valueOf(maxZ));

        cmd.add("--map");
        cmd.add(cleanMap);

        cmd.add("--name");
        cmd.add(cleanMap);

        cmd.add("--category");
        cmd.add(cleanCategory);

        cmd.add("--lang");
        cmd.add(lang);

        cmd.add("--lighting-style");
        cmd.add(cleanStyle);

        cmd.add("--dimension");
        cmd.add(resolvedDimension);

        if (mcPhysics) {
            cmd.add("--mc-physics");
        } else {
            cmd.add("--no-mc-physics");
        }

        if (hqLight) {
            cmd.add("--hq-light");
        }

        if (importGMod) {
            cmd.add("--import-to-gmod");
        } else {
            cmd.add("--no-import-to-gmod");
        }

        if (exportVmf) {
            cmd.add("--export-vmf");
        }

        if (!EXPORT_RUNNING.compareAndSet(false, true)) {
            boolean isRu = "ru".equals(lang);
            String shortErr = isRu
                ? "Экспорт уже выполняется. Дождитесь завершения текущего процесса."
                : "An export is already running. Wait for it to finish.";
            setLastLaunchError(shortErr);
            sendChat(client, "§e[MCGM] " + shortErr);
            return false;
        }

        BufferedWriter logWriter = null;
        try {
            File tempDir = new File(workspaceDir, "temp");
            if (!tempDir.exists()) {
                tempDir.mkdirs();
            }
            File logFile = new File(tempDir, "export.log");
            logWriter = Files.newBufferedWriter(
                logFile.toPath(),
                StandardCharsets.UTF_8,
                StandardOpenOption.CREATE,
                StandardOpenOption.WRITE,
                StandardOpenOption.TRUNCATE_EXISTING
            );
            logWriter.write("=== MCGM Export Log: map=" + cleanMap + " | dim=" + resolvedDimension +
                " | style=" + cleanStyle + " | mc_physics=" + mcPhysics + " | hq_light=" + hqLight +
                " | world=" + worldPath + " ===");
            logWriter.newLine();
            logWriter.flush();

            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(workspaceDir);
            pb.redirectErrorStream(true);
            Process process = pb.start();

            if (client != null && client.player != null) {
                boolean isRu = "ru".equals(lang);
                String startMsg = isRu
                    ? "§a§l[MCGM]§r §aЭкспорт запущен для карты §e\"" + cleanMap + "\"§a (§b" + resolvedDimension +
                        "§a, свет: §f" + cleanStyle + "§a)! Координаты: (" +
                        minX + "," + minY + "," + minZ + ") - (" + maxX + "," + maxY + "," + maxZ + ")"
                    : "§a§l[MCGM]§r §aExport started for map §e\"" + cleanMap + "\"§a (§b" + resolvedDimension +
                        "§a, light: §f" + cleanStyle + "§a)! Coords: (" +
                        minX + "," + minY + "," + minZ + ") to (" + maxX + "," + maxY + "," + maxZ + ")";
                client.player.sendMessage(Text.literal(startMsg), false);
            }

            streamProcessToChat(client, process, logWriter, cleanMap, lang);
            return true;
        } catch (Exception e) {
            EXPORT_RUNNING.set(false);
            if (logWriter != null) {
                try {
                    logWriter.close();
                } catch (IOException ignored) {
                }
            }
            e.printStackTrace();
            boolean isRu = "ru".equals(lang);
            String shortErr = (isRu ? "Не удалось запустить экспорт: " : "Failed to launch export: ") + e.getMessage();
            setLastLaunchError(shortErr);
            sendChat(client, "§c[MCGM] " + shortErr);
            return false;
        }
    }

    private static boolean isNoisyLogLine(String trimmed) {
        if (trimmed.isEmpty()) return true;
        if (PAT_NOISY_SEP.matcher(trimmed).matches()) return true;
        if (trimmed.startsWith("minecraft:") && trimmed.contains(":")) return true;
        if (trimmed.equals("--- Detected Unique Blocks ---") || trimmed.equals("------------------------------")) return true;
        if (trimmed.startsWith("Processing ") || trimmed.startsWith("qdir: ") || trimmed.startsWith("gamedir: ") || trimmed.startsWith("input file")) return true;
        if (trimmed.startsWith("writing ") || trimmed.startsWith("reading ") || trimmed.startsWith("model ")) return true;
        if (trimmed.startsWith("Valve Software") || trimmed.startsWith("materialPath:") || trimmed.startsWith("Loading ") || trimmed.startsWith("Finding ") || trimmed.startsWith("Patching ")) return true;
        if (trimmed.startsWith("numthreads") || trimmed.startsWith("BuildVisLeafs") || trimmed.startsWith("chop ") || trimmed.startsWith("0...1...2...")) return true;
        if (trimmed.startsWith("Fra:") || trimmed.startsWith("Read blend:") || trimmed.startsWith("Saved:") || trimmed.startsWith("Blender ") || trimmed.startsWith("VTFCmd") || trimmed.startsWith("Creating texture") || trimmed.equals("Done.")) return true;
        if (trimmed.startsWith("Compiled Model Files:") || trimmed.startsWith("Compiled Material Files:") || trimmed.startsWith("Compiled Map Files:") || trimmed.startsWith("Output Addon Path:")) return true;
        if (trimmed.startsWith("- ") && (trimmed.contains("\\") || trimmed.contains("/"))) return true;
        return false;
    }

    private static void streamProcessToChat(
        MinecraftClient client,
        Process process,
        BufferedWriter ownedLogWriter,
        String mapName,
        String lang
    ) {
        Thread readerThread = new Thread(() -> {
            boolean isRussian = "ru".equalsIgnoreCase(lang);
            long lastInfoEmitMs = 0L;
            long lastWarnEmitMs = 0L;
            String lastMilestone = "";
            String lastInfoLine = "";
            String lastWarnLine = "";
            int errorBurst = 0;
            int warningCount = 0;
            int suppressedInfoCount = 0;

            try (
                BufferedReader reader = new BufferedReader(new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8));
                BufferedWriter logWriter = ownedLogWriter
            ) {
                long lastFlushMs = System.currentTimeMillis();
                String line;
                while ((line = reader.readLine()) != null) {
                    long now = System.currentTimeMillis();
                    if (logWriter != null) {
                        logWriter.write(line);
                        logWriter.newLine();
                    }

                    String trimmed = line.trim();
                    if (isNoisyLogLine(trimmed)) {
                        if (logWriter != null && (now - lastFlushMs) >= 250L) {
                            logWriter.flush();
                            lastFlushMs = now;
                        }
                        continue;
                    }

                    String chatMsg = null;

                    boolean isMilestone = PAT_PCT_START.matcher(trimmed).matches()
                        || trimmed.contains("[100%]")
                        || trimmed.startsWith("Status: SUCCESS")
                        || trimmed.startsWith("Playable Map:")
                        || trimmed.contains("Export completed");

                    boolean isError = trimmed.startsWith("Error:")
                        || trimmed.startsWith("Error ")
                        || trimmed.startsWith("[Error]")
                        || trimmed.startsWith("Traceback")
                        || trimmed.startsWith("Exception")
                        || trimmed.contains("FAILED");

                    boolean isWarning = trimmed.startsWith("Warning:")
                        || trimmed.startsWith("WARN:")
                        || trimmed.startsWith("[Pipeline] Warning")
                        || trimmed.startsWith("[MapCompiler] Warning");

                    boolean isTaggedInfo = trimmed.startsWith("[Pipeline]")
                        || trimmed.startsWith("[ModdedExporter]")
                        || trimmed.startsWith("[MapGenerator]")
                        || trimmed.startsWith("[Compiler]")
                        || trimmed.startsWith("[Hammer]");

                    if (logWriter != null && (isMilestone || isError || (now - lastFlushMs) >= 250L)) {
                        logWriter.flush();
                        lastFlushMs = now;
                    }

                    if (isMilestone) {
                        errorBurst = 0;
                        if (!trimmed.equals(lastMilestone)) {
                            lastMilestone = trimmed;
                            if (trimmed.contains("[100%]") || trimmed.startsWith("Status: SUCCESS") || trimmed.contains("Export completed")) {
                                chatMsg = "§a§l[MCGM]§r §a" + trimmed;
                            } else if (PAT_PCT_ANY.matcher(trimmed).matches()) {
                                chatMsg = "§6§l[MCGM]§r §e" + trimmed;
                            } else {
                                chatMsg = "§b[MCGM]§r §f" + trimmed;
                            }
                        }
                    } else if (isError) {
                        errorBurst++;
                        if (errorBurst <= 4) {
                            chatMsg = "§c§l[MCGM]§r §c" + trimmed;
                        } else if (errorBurst == 5) {
                            chatMsg = isRussian
                                ? "§c[MCGM] ... подробности ошибки записаны в temp/export.log"
                                : "§c[MCGM] ... additional error details written to temp/export.log";
                        }
                    } else if (isWarning) {
                        warningCount++;
                        if (!trimmed.equals(lastWarnLine) && (now - lastWarnEmitMs >= 2000L)) {
                            lastWarnLine = trimmed;
                            lastWarnEmitMs = now;
                            chatMsg = "§e§l[MCGM]§r §6" + trimmed;
                        } else {
                            suppressedInfoCount++;
                        }
                    } else if (isTaggedInfo) {
                        if (!trimmed.equals(lastInfoLine) && (now - lastInfoEmitMs >= 1500L)) {
                            lastInfoLine = trimmed;
                            lastInfoEmitMs = now;
                            chatMsg = "§b[MCGM]§r §f" + trimmed;
                        } else {
                            suppressedInfoCount++;
                        }
                    } else {
                        suppressedInfoCount++;
                    }

                    if (chatMsg != null && client != null) {
                        final String finalMsg = chatMsg;
                        client.execute(() -> sendChat(client, finalMsg));
                    }
                }

                int exitCode = process.waitFor();
                final int finalWarnings = warningCount;
                final int finalSuppressed = suppressedInfoCount;
                if (client != null) {
                    client.execute(() -> {
                        if (client.player != null) {
                            if (exitCode == 0) {
                                String extra = "";
                                if (finalWarnings > 0) {
                                    extra = isRussian
                                        ? "\n§7(Предупреждений: " + finalWarnings + ", подробный лог: temp/export.log)"
                                        : "\n§7(Warnings: " + finalWarnings + ", full log: temp/export.log)";
                                } else if (finalSuppressed > 0) {
                                    extra = isRussian
                                        ? "\n§7(Подробный лог: temp/export.log)"
                                        : "\n§7(Full log: temp/export.log)";
                                }
                                String msg = isRussian
                                    ? "§a§l[MCGM]§r §aЭкспорт успешно завершен! Карта §e\"" + mapName + "\"§a готова к игре в Garry's Mod.\n§fЗапуск в консоли GMod: §fmap " + mapName + extra
                                    : "§a§l[MCGM]§r §aExport completed successfully! Map §e\"" + mapName + "\"§a is ready in Garry's Mod.\n§fLaunch in GMod console: §fmap " + mapName + extra;
                                client.player.sendMessage(Text.literal(msg), false);
                            } else {
                                String msg = isRussian
                                    ? "§c§l[MCGM]§r §cЭкспорт завершился с ошибкой (код " + exitCode + "). Подробности: temp/export.log"
                                    : "§c§l[MCGM]§r §cExport failed with exit code " + exitCode + ". Check temp/export.log for details.";
                                client.player.sendMessage(Text.literal(msg), false);
                            }
                        }
                    });
                }
            } catch (Exception e) {
                e.printStackTrace();
                if (client != null) {
                    client.execute(() -> {
                        if (client.player != null) {
                            String errMsg = isRussian
                                ? "§c[MCGM] Ошибка чтения вывода: " + e.getMessage()
                                : "§c[MCGM] Failed to read export output: " + e.getMessage();
                            client.player.sendMessage(Text.literal(errMsg), false);
                        }
                    });
                }
            } finally {
                EXPORT_RUNNING.set(false);
            }
        }, "MCGM-Process-Streamer");
        readerThread.setDaemon(true);
        readerThread.start();
    }

    public static boolean launchGui(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category
    ) {
        return launchGui(client, minX, minY, minZ, maxX, maxY, maxZ, mapName, category, null, "auto");
    }

    public static boolean launchGui(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        String customMcgmPath
    ) {
        return launchGui(client, minX, minY, minZ, maxX, maxY, maxZ, mapName, category, customMcgmPath, "auto");
    }

    public static boolean launchGui(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        String customMcgmPath,
        String explicitLang
    ) {
        ScreenSettings saved = loadScreenSettings(client);
        return launchGui(
            client,
            minX, minY, minZ,
            maxX, maxY, maxZ,
            mapName,
            category,
            saved.lightingStyle,
            saved.mcPhysics,
            saved.hqLight,
            saved.dimensionMode,
            saved.worldOverride,
            customMcgmPath,
            explicitLang
        );
    }

    public static boolean launchGui(
        MinecraftClient client,
        int minX, int minY, int minZ,
        int maxX, int maxY, int maxZ,
        String mapName,
        String category,
        String lightingStyle,
        boolean mcPhysics,
        boolean hqLight,
        String dimensionMode,
        String worldOverride,
        String customMcgmPath,
        String explicitLang
    ) {
        setLastLaunchError("");
        String resolvedPath;
        String trimmedCustom = customMcgmPath != null ? customMcgmPath.trim() : "";
        if (!trimmedCustom.isEmpty()) {
            File customExe = resolveMcgmExecutable(new File(trimmedCustom));
            if (customExe != null) {
                resolvedPath = customExe.getAbsolutePath();
                saveMcgmPath(client, resolvedPath);
            } else {
                resolvedPath = trimmedCustom;
            }
        } else {
            resolvedPath = getOrResolveMcgmPath(client);
        }
        File exeOrDir = new File(resolvedPath);
        File exeFile = resolveMcgmExecutable(exeOrDir);
        File workspaceDir = (exeFile != null) ? exeFile.getParentFile() : exeOrDir.getParentFile();

        if (workspaceDir == null || !workspaceDir.isDirectory()) {
            workspaceDir = (client != null && client.runDirectory != null) ? client.runDirectory : new File(".");
        }

        String cleanMap = sanitizeMapName(mapName);
        String cleanCategory = sanitizeCategory(category);
        String lang = resolveLang(client, explicitLang);
        String cleanStyle = sanitizeLightingStyle(lightingStyle);
        String resolvedDimension = resolveDimension(client, dimensionMode);

        List<String> cmd = new ArrayList<>();
        if (exeFile != null && exeFile.isFile()) {
            cmd.add(exeFile.getAbsolutePath());
        } else {
            boolean isRu = "ru".equals(lang);
            String shortErr = isRu
                ? "mcgm.exe не найден: \"" + resolvedPath + "\""
                : "mcgm.exe not found at: \"" + resolvedPath + "\"";
            setLastLaunchError(shortErr);
            String errMsg = isRu
                ? "§c§l[MCGM]§r §cОшибка: mcgm.exe не найден по пути: §f\"" + resolvedPath + "\"§c!" +
                    "\n§eУкажите правильный путь в поле 'Путь к mcgm.exe' в меню /mcgm или в config/mcgm.json"
                : "§c§l[MCGM]§r §cError: mcgm.exe not found at: §f\"" + resolvedPath + "\"§c!" +
                    "\n§eSet the valid path in the 'Path to mcgm.exe' field in /mcgm or in config/mcgm.json";
            sendChat(client, errMsg);
            return false;
        }
        cmd.add("--gui");

        String worldPath = resolveWorldForLaunch(client, worldOverride, lang, false);
        if (worldPath == null) {
            return false;
        }
        if (!worldPath.isEmpty()) {
            cmd.add("--world");
            cmd.add(worldPath);
        }

        cmd.add("--coords");
        cmd.add(String.valueOf(minX));
        cmd.add(String.valueOf(minY));
        cmd.add(String.valueOf(minZ));
        cmd.add(String.valueOf(maxX));
        cmd.add(String.valueOf(maxY));
        cmd.add(String.valueOf(maxZ));

        cmd.add("--map");
        cmd.add(cleanMap);

        cmd.add("--name");
        cmd.add(cleanMap);

        cmd.add("--category");
        cmd.add(cleanCategory);

        cmd.add("--lang");
        cmd.add(lang);

        cmd.add("--lighting-style");
        cmd.add(cleanStyle);

        cmd.add("--dimension");
        cmd.add(resolvedDimension);

        if (mcPhysics) {
            cmd.add("--mc-physics");
        } else {
            cmd.add("--no-mc-physics");
        }

        if (hqLight) {
            cmd.add("--hq-light");
        }

        try {
            File tempDir = new File(workspaceDir, "temp");
            if (!tempDir.exists()) {
                tempDir.mkdirs();
            }
            File logFile = new File(tempDir, "gui.log");

            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(workspaceDir);
            pb.redirectErrorStream(true);
            pb.redirectOutput(ProcessBuilder.Redirect.appendTo(logFile));
            pb.start();

            if (client != null && client.player != null) {
                boolean isRu = "ru".equals(lang);
                String openMsg = isRu
                    ? "§b[MCGM] Открыт интерфейс программы MCGM с картой §e\"" + cleanMap + "\"§b и координатами."
                    : "§b[MCGM] Opened MCGM GUI with map §e\"" + cleanMap + "\"§b and preloaded coordinates.";
                client.player.sendMessage(Text.literal(openMsg), false);
            }
            return true;
        } catch (Exception e) {
            e.printStackTrace();
            boolean isRu = "ru".equals(lang);
            String shortErr = (isRu ? "Не удалось открыть GUI: " : "Failed to launch GUI: ") + e.getMessage();
            setLastLaunchError(shortErr);
            sendChat(client, "§c[MCGM] " + shortErr);
            return false;
        }
    }
}
