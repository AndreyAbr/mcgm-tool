# MCGM (Minecraft to Garry's Mod)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-Native-orange.svg)](https://www.rust-lang.org/)
[![Minecraft](https://img.shields.io/badge/Minecraft-1.18%20%E2%80%93%2026.x%2B-green.svg)](https://www.minecraft.net/)
[![Source Engine](https://img.shields.io/badge/Garry's%20Mod-Source%20Engine-blue.svg)](https://gmod.facepunch.com/)

**MCGM** переносит постройки и участки миров Minecraft Java Edition (`1.18 – 26.x+`) в готовые карты Garry's Mod (`.bsp`) с текстурами, освещением, табличками, дверями, кнопками, анимацией воды/огня и физикой движения из Minecraft. Работает **только на Windows 10+**; Blender и Python не нужны.

## Что понадобится

| Инструмент | Ссылка | Статус | Куда поставить / как находится |
|---|---|---|---|
| **Garry's Mod** | [Steam](https://store.steampowered.com/app/4000/Garrys_Mod/) | Обязательно | Находится автоматически через Steam (`studiomdl.exe`, `vbsp`/`vvis`/`vrad`, `garrysmod/`). |
| **VTFLib (`VTFCmd.exe`)** | [Скачать (Nem's Tools)](https://nemstools.github.io/pages/VTFLib-Download.html) · [GitHub](https://github.com/NeilJed/VTFLib) | Обязательно | Распакуйте в `C:\Tools\VTFLib\` или рядом с `mcgm.exe` в `tools\VTFLib\`. |
| **Minecraft: Java Edition** | [minecraft.net](https://www.minecraft.net/) / [PrismLauncher](https://prismlauncher.org/) / [MultiMC](https://multimc.org/) | Обязательно | Установленный клиент нужной версии (клиентские `.jar`, профили, инстансы и папки `saves/` находятся автоматически). |
| **Tools++ (`vbsp++`, `vvis++`, `vrad++`)** | [GitHub Releases](https://github.com/ficool2/misc_tools/releases) | По желанию | Сильно ускоряет сборку (есть GPU-свет на RTX 2000+, RX 6000+, Intel Arc). Распакуйте в `C:\Tools\Tools++\` или рядом с `mcgm.exe` в `Tools++\`. |

> **Подсказка:** `mcgm.exe` запускается без окна командной строки. Для CLI-команд в PowerShell добавляйте `| Out-Host`, чтобы дождаться вывода: `.\mcgm.exe --check-env | Out-Host`, `.\mcgm.exe --list-instances | Out-Host`, `.\mcgm.exe --detect-tools | Out-Host`.

## Как перенести карту

1. Запустите `mcgm.exe` (откроется только окно программы, без консоли).
2. **1. Мир:** выберите инстанс/профиль лаунчера (Vanilla `.minecraft`, `versions/*/saves`, Prism Launcher, MultiMC или добавьте свою папку кнопкой **+ Папка...**) и нужный мир из списка. Кнопка **Обзор...** открывает стандартный проводник Windows сразу в папке `saves/` выбранного инстанса.
3. **2. Область:** выберите размер области вокруг спавна (**±32**, **±64**, **±128**) или введите координаты **Мин / Макс**.
4. **3. Карта:** проверьте имя карты, качество и свет, затем нажмите **Создать карту**. После завершения нажмите **Открыть GMod** (или введите `map <имя_карты>` в консоли игры).

> Дополнительные настройки (измерение, масштаб, папка вывода, сохранение `.vmf` для Hammer, лог сборки) открываются кнопками **Дополнительно** и **Журнал** внизу окна.

## Физика Minecraft в Garry's Mod

Вместе с картой автоматически устанавливается аддон с физикой передвижения из Minecraft: шаг на 1 блок (или автопрыжок), бег, карабканье по лестницам и лианам, скольжение по льду, прыгучая слизь, вязкий мёд и эффекты воды/лавы.

- Включить или выключить физику прямо во время игры: напишите `!mcphysics` в чат или `mcgm_toggle_physics` в консоль.
- Тонкая настройка параметров доступна в **Q-меню → MCGM**.

## Мод для выделения в Minecraft (по желанию, Fabric 1.20.1)

Мод необязателен: `mcgm.exe` работает самостоятельно с любыми мирами `1.18 – 26.x+`.

| Клавиша / Команда | Что делает |
|---|---|
| `[` | Поставить первую точку (`Pos 1`) |
| `]` | Поставить вторую точку (`Pos 2`) |
| `M` | Открыть меню настроек MCGM |
| `/mcgm export` | Сразу запустить экспорт выделенного участка |
| `/mcgm gui` | Открыть окно MCGM |

<details>
<summary>Консольный режим (CLI) и сборка из исходников</summary>

Для сборки из исходников понадобятся [Rust (rustup.rs)](https://rustup.rs/) и [JDK 17+ (Adoptium)](https://adoptium.net/) (только если собираете мод для Minecraft).

```powershell
# Проверка и сборка mcgm.exe
cargo test
cargo build --release

# Диагностика окружения и список найденных инстансов
# (mcgm.exe использует GUI-подсистему без окна консоли, поэтому в PowerShell используйте | Out-Host)
.\target\release\mcgm.exe --check-env | Out-Host
.\target\release\mcgm.exe --list-instances | Out-Host

# Сборка и установка мода Fabric 1.20.1
powershell -ExecutionPolicy Bypass -File minecraft_mod\build.ps1
```

Пример экспорта через консоль (по пути к миру или по имени инстанса):
```powershell
.\target\release\mcgm.exe --world "C:\Users\<User>\AppData\Roaming\.minecraft\saves\World" `
  --coords 100 64 100 228 140 228 `
  --name my_build `
  --category mcgm `
  --map gm_minecraft_build `
  --import-to-gmod | Out-Host

.\target\release\mcgm.exe --instance "26.2" `
  --coords -275 61 29 -215 111 89 `
  --preset preview | Out-Host
```

Основные флаги CLI:
- `--lighting-style <minecraft|gmod|bright|night>` — пресет освещения и неба.
- `--preset <preview|balanced|hq>` — пресет качества сборки (управляет `-fast`/VRAD/кэшем).
- `--fast-vrad` / `--hq-light` — быстрый расчёт света или повышенное качество (полный VVIS).
- `--collision-mode <per_chunk_phy|vmf_brushes>` — коллизии моделями или VMF-брашами (то же, что `--vmf-brushes`).
- `--chunk-size <N>` — размер чанка при экспорте (по умолчанию `64`).
- `--max-workers <N>` — количество потоков для компиляции моделей.
- `--reuse-cache` — не перекомпилировать уже готовые модели при повторном экспорте.
- `--export-vmf` — сохранить исходный `.vmf` файл карты.
- `--no-mc-physics` — не устанавливать аддон физики Minecraft.
- `--no-import-to-gmod` — сохранить результат только в папку вывода, не копируя в `garrysmod/`.
- `--gui` — принудительно открыть окно программы; `--auto-start` — запустить экспорт сразу после открытия.
- `--list-instances` — вывести список найденных инстансов Vanilla / Prism Launcher / MultiMC и их папок `saves` (поддерживает `--json`).
- `--instance <имя>` — выбрать инстанс по имени для разрешения `--world` (или автоматического выбора самого свежего мира).
- `--detect-tools` — найти внешние утилиты и обновить `config.json`; `--check-env` — диагностика окружения.

</details>

<details>
<summary>Конфигурация (config.json)</summary>

Файл `config.json` создаётся и обновляется командой `--detect-tools`. Поддерживаемые ключи:
- `studiomdl_path` — путь к `studiomdl.exe` (Garry's Mod SDK).
- `vbsp_path`, `vvis_path`, `vrad_path` — пути к компиляторам карты (`vbsp++.exe`/`vbsp.exe` и т.д.).
- `vtfcmd_path` — путь к `VTFCmd.exe` (VTFLib).
- `game_dir` — путь к папке `garrysmod/`.
- `hammer_path` — путь к `hammer.exe` (опционально).
- `vanilla_jar_path` — путь к клиентскому `.jar` Minecraft для извлечения ресурсов.
- `world_path` — путь к папке сохранений Minecraft (`saves/`) или конкретному миру.
- `instance_dirs` — (опционально) массив дополнительных путей к папкам лаунчеров, `instances/` или `saves/`.
- `scale_factor` — масштаб (единиц Source на 1 блок Minecraft, по умолчанию `39.37`).
- `max_convex_pieces` — максимум выпуклых частей коллизии на модель (по умолчанию `64`).

</details>

## Лицензия

[MIT](LICENSE)
