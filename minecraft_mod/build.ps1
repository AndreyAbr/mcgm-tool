$ErrorActionPreference = "Stop"

Write-Host "=========================================================="
Write-Host " Building mcgm-bridge Fabric 1.20.1 Client Mod            "
Write-Host "=========================================================="

# Locate JDK 17+ (javac, jar, java)
$javac = $null
$jar = $null
$java = $null

$jdkCandidates = @()
if ($env:APPDATA) {
    $jdkCandidates += Join-Path $env:APPDATA ".minecraft\runtime\java-runtime-gamma\windows\java-runtime-gamma\bin"
}
if ($env:JAVA_HOME) {
    $jdkCandidates += Join-Path $env:JAVA_HOME "bin"
}
if ($env:JDK_HOME) {
    $jdkCandidates += Join-Path $env:JDK_HOME "bin"
}

foreach ($cand in $jdkCandidates) {
    $jc = Join-Path $cand "javac.exe"
    if (Test-Path $jc) {
        $javac = $jc
        $jar = Join-Path $cand "jar.exe"
        $java = Join-Path $cand "java.exe"
        break
    }
}

if (-not $javac) {
    foreach ($baseDir in @("C:\Program Files\Eclipse Adoptium", "C:\Program Files\Java", "C:\Program Files\Microsoft", "C:\Program Files\BellSoft", "C:\Program Files\Zulu")) {
        if (Test-Path $baseDir) {
            $foundJc = Get-ChildItem -Path $baseDir -Recurse -Filter "javac.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($foundJc) {
                $javac = $foundJc.FullName
                $binDir = Split-Path $javac
                $jar = Join-Path $binDir "jar.exe"
                $java = Join-Path $binDir "java.exe"
                break
            }
        }
    }
}

if (-not $javac) {
    $cmdJavac = Get-Command javac.exe -ErrorAction SilentlyContinue
    if ($cmdJavac) {
        $javac = $cmdJavac.Source
        $binDir = Split-Path $javac
        $jar = Join-Path $binDir "jar.exe"
        $java = Join-Path $binDir "java.exe"
    }
}

if (-not $javac -or -not (Test-Path $javac)) {
    Write-Host "==========================================================" -ForegroundColor Red
    Write-Host "[ERROR] JDK 17+ (javac.exe) not found on this computer!" -ForegroundColor Red
    Write-Host "Please install OpenJDK 17 (e.g. https://adoptium.net/)" -ForegroundColor Yellow
    Write-Host "or launch Minecraft 1.20.1 in the official launcher once to download the runtime." -ForegroundColor Yellow
    Write-Host "==========================================================" -ForegroundColor Red
    exit 1
}

function Download-FileIfMissing {
    param(
        [string]$Url,
        [string]$DestPath
    )
    if ((Test-Path $DestPath) -and ((Get-Item $DestPath).Length -gt 0)) {
        return
    }
    $parent = Split-Path $DestPath -Parent
    if (-not (Test-Path $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $tmp = "$DestPath.tmp"
    if (Test-Path $tmp) { Remove-Item -Force $tmp }
    Write-Host "  Downloading $Url -> $(Split-Path $DestPath -Leaf)"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $Url -OutFile $tmp -UseBasicParsing
    Move-Item -Path $tmp -Destination $DestPath -Force
}

function Extract-ZipEntry {
    param(
        [string]$ZipPath,
        [string]$EntryName,
        [string]$DestPath
    )
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        $entry = $zip.GetEntry($EntryName)
        if (-not $entry) {
            throw "Entry '$EntryName' not found in '$ZipPath'"
        }
        $parent = Split-Path $DestPath -Parent
        if (-not (Test-Path $parent)) {
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
        }
        $inStream = $entry.Open()
        $outStream = [System.IO.File]::Create($DestPath)
        try {
            $inStream.CopyTo($outStream)
        } finally {
            $outStream.Dispose()
            $inStream.Dispose()
        }
    } finally {
        $zip.Dispose()
    }
}

$projectDir = $PSScriptRoot
$srcJavaDir = Join-Path $projectDir "src\main\java"
$srcResDir = Join-Path $projectDir "src\main\resources"
$buildDir = Join-Path $projectDir "build"
$classesDir = Join-Path $buildDir "classes"
$libsDir = Join-Path $buildDir "libs"

$cacheDir = if ($env:MCGM_MOD_CACHE) {
    $env:MCGM_MOD_CACHE
} elseif ($env:LOCALAPPDATA) {
    Join-Path $env:LOCALAPPDATA "mcgm\mod-cache\1.20.1"
} else {
    Join-Path $env:TEMP "mcgm-mod-cache\1.20.1"
}

$cacheLibsDir = Join-Path $cacheDir "libs"
$cacheFapiIntermDir = Join-Path $cacheDir "fapi_intermediary"
$cacheFapiNamedDir = Join-Path $cacheDir "fapi_named"
$tinyRemapperJar = Join-Path $cacheDir "tiny-remapper.jar"
$intermTiny = Join-Path $cacheDir "intermediary-1.20.1.tiny"
$yarnTiny = Join-Path $cacheDir "yarn-mappings.tiny"
$officialMcJar = Join-Path $cacheDir "minecraft-1.20.1-official.jar"
$intermMcJar = Join-Path $cacheDir "minecraft-1.20.1-intermediary.jar"
$namedMc = Join-Path $cacheDir "minecraft-1.20.1-named.jar"
$readyMarker = Join-Path $cacheDir ".ready"

$needsBootstrap = -not (
    (Test-Path $readyMarker) -and
    (Test-Path $tinyRemapperJar) -and
    (Test-Path $yarnTiny) -and
    (Test-Path $namedMc) -and
    (Test-Path $cacheFapiNamedDir) -and
    ((Get-ChildItem -Path $cacheFapiNamedDir -Filter "*.jar" -ErrorAction SilentlyContinue).Count -gt 0) -and
    ((Get-ChildItem -Path $cacheLibsDir -Filter "*.jar" -ErrorAction SilentlyContinue).Count -gt 0)
)

if ($needsBootstrap) {
    Write-Host "Bootstrapping Minecraft 1.20.1 + Fabric API build dependencies into $cacheDir..."
    New-Item -ItemType Directory -Path $cacheDir -Force | Out-Null
    New-Item -ItemType Directory -Path $cacheLibsDir -Force | Out-Null
    New-Item -ItemType Directory -Path $cacheFapiIntermDir -Force | Out-Null
    New-Item -ItemType Directory -Path $cacheFapiNamedDir -Force | Out-Null

    # 1. TinyRemapper
    Download-FileIfMissing `
        -Url "https://maven.fabricmc.net/net/fabricmc/tiny-remapper/0.10.4/tiny-remapper-0.10.4-fat.jar" `
        -DestPath $tinyRemapperJar

    # 2. Intermediary mappings (official -> intermediary)
    $intermJar = Join-Path $cacheDir "intermediary-1.20.1-v2.jar"
    Download-FileIfMissing `
        -Url "https://maven.fabricmc.net/net/fabricmc/intermediary/1.20.1/intermediary-1.20.1-v2.jar" `
        -DestPath $intermJar
    Extract-ZipEntry -ZipPath $intermJar -EntryName "mappings/mappings.tiny" -DestPath $intermTiny

    # 3. Yarn mappings (intermediary -> named)
    $yarnJar = Join-Path $cacheDir "yarn-1.20.1+build.10-v2.jar"
    Download-FileIfMissing `
        -Url "https://maven.fabricmc.net/net/fabricmc/yarn/1.20.1+build.10/yarn-1.20.1+build.10-v2.jar" `
        -DestPath $yarnJar
    Extract-ZipEntry -ZipPath $yarnJar -EntryName "mappings/mappings.tiny" -DestPath $yarnTiny

    # 4. Mojang 1.20.1 manifest + client JAR + Java libraries
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Write-Host "  Fetching Mojang version manifest..."
    $manifest = Invoke-RestMethod -Uri "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json" -UseBasicParsing
    $verEntry = $manifest.versions | Where-Object { $_.id -eq "1.20.1" } | Select-Object -First 1
    if (-not $verEntry) {
        throw "Minecraft 1.20.1 not found in Mojang version manifest."
    }
    $verJson = Invoke-RestMethod -Uri $verEntry.url -UseBasicParsing

    Download-FileIfMissing -Url $verJson.downloads.client.url -DestPath $officialMcJar

    # Fabric Loader
    Download-FileIfMissing `
        -Url "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar" `
        -DestPath (Join-Path $cacheLibsDir "fabric-loader-0.15.11.jar")

    # Non-native Minecraft compile libraries
    foreach ($lib in $verJson.libraries) {
        if ($lib.downloads -and $lib.downloads.artifact -and $lib.downloads.artifact.url) {
            $artPath = $lib.downloads.artifact.path
            if ($artPath -match "natives") { continue }
            $fileName = Split-Path $artPath -Leaf
            Download-FileIfMissing -Url $lib.downloads.artifact.url -DestPath (Join-Path $cacheLibsDir $fileName)
        }
    }

    $depLibs = Get-ChildItem -Path $cacheLibsDir -Filter "*.jar" | ForEach-Object { $_.FullName }

    # 5. Remap Minecraft 1.20.1: official -> intermediary -> named
    if (-not (Test-Path $intermMcJar)) {
        Write-Host "  Remapping Minecraft 1.20.1 (official -> intermediary)..."
        $tmpInterm = "$intermMcJar.tmp"
        & $java -jar $tinyRemapperJar $officialMcJar $tmpInterm $intermTiny official intermediary @depLibs
        if ($LASTEXITCODE -ne 0) {
            throw "tiny-remapper (official -> intermediary) failed with exit code $LASTEXITCODE"
        }
        Move-Item -Path $tmpInterm -Destination $intermMcJar -Force
    }
    if (-not (Test-Path $namedMc)) {
        Write-Host "  Remapping Minecraft 1.20.1 (intermediary -> named)..."
        $tmpNamed = "$namedMc.tmp"
        & $java -jar $tinyRemapperJar $intermMcJar $tmpNamed $yarnTiny intermediary named @depLibs
        if ($LASTEXITCODE -ne 0) {
            throw "tiny-remapper (intermediary -> named) failed with exit code $LASTEXITCODE"
        }
        Move-Item -Path $tmpNamed -Destination $namedMc -Force
    }

    # 6. Download Fabric API 0.92.6+1.20.1, extract nested module JARs, remap to named
    $fapiBundle = Join-Path $cacheDir "fabric-api-0.92.6+1.20.1.jar"
    Download-FileIfMissing `
        -Url "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/0.92.6+1.20.1/fabric-api-0.92.6+1.20.1.jar" `
        -DestPath $fapiBundle

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $fapiZip = [System.IO.Compression.ZipFile]::OpenRead($fapiBundle)
    try {
        foreach ($entry in $fapiZip.Entries) {
            if ($entry.FullName -like "META-INF/jars/*.jar") {
                $modJarName = Split-Path $entry.FullName -Leaf
                $destInterm = Join-Path $cacheFapiIntermDir $modJarName
                if (-not (Test-Path $destInterm)) {
                    $inS = $entry.Open()
                    $outS = [System.IO.File]::Create($destInterm)
                    try { $inS.CopyTo($outS) } finally { $outS.Dispose(); $inS.Dispose() }
                }
            }
        }
    } finally {
        $fapiZip.Dispose()
    }

    $allIntermFapi = Get-ChildItem -Path $cacheFapiIntermDir -Filter "*.jar" | ForEach-Object { $_.FullName }
    Write-Host "  Remapping $($allIntermFapi.Count) Fabric API modules (intermediary -> named)..."
    foreach ($modInterm in $allIntermFapi) {
        $modName = Split-Path $modInterm -Leaf
        $modNamed = Join-Path $cacheFapiNamedDir $modName
        if (-not (Test-Path $modNamed)) {
            $tmpMod = "$modNamed.tmp"
            $otherInterm = $allIntermFapi | Where-Object { $_ -ne $modInterm }
            $remapCp = @($intermMcJar) + $otherInterm + $depLibs
            & $java -jar $tinyRemapperJar $modInterm $tmpMod $yarnTiny intermediary named @remapCp
            if ($LASTEXITCODE -ne 0) {
                throw "tiny-remapper failed for $modName with exit code $LASTEXITCODE"
            }
            Move-Item -Path $tmpMod -Destination $modNamed -Force
        }
    }

    Set-Content -Path $readyMarker -Value "1.20.1-fapi-0.92.6" -Encoding ASCII
    Write-Host "Dependency cache ready."
}

$appDataDir = if ($env:APPDATA) { $env:APPDATA } else { "C:\Users\Default\AppData\Roaming" }
$modsDir = Join-Path $appDataDir ".minecraft\mods"

# Clean build directory
if (Test-Path $classesDir) {
    Remove-Item -Recurse -Force $classesDir
}
New-Item -ItemType Directory -Path $classesDir -Force | Out-Null
New-Item -ItemType Directory -Path $libsDir -Force | Out-Null

# Assemble classpath from external cache
$libs = Get-ChildItem -Path $cacheLibsDir -Filter "*.jar" | ForEach-Object { $_.FullName }
$fapi = Get-ChildItem -Path $cacheFapiNamedDir -Filter "*.jar" | ForEach-Object { $_.FullName }
$allCp = @($namedMc) + $fapi + $libs

# Collect Java source files
$javaFiles = Get-ChildItem -Path $srcJavaDir -Recurse -Filter "*.java" | ForEach-Object { $_.FullName }
Write-Host "Found $($javaFiles.Count) Java source files."

# Create javac argument file to prevent Windows command line length limit issues
$argFile = Join-Path $buildDir "javac_args.txt"
$cpArg = ($allCp | ForEach-Object { $_.Replace('\', '/') }) -join ";"
$classesArg = $classesDir.Replace('\', '/')
$argContent = @(
    "-cp",
    "`"$cpArg`"",
    "-d",
    "`"$classesArg`"",
    "-source", "17",
    "-target", "17",
    "-encoding", "UTF-8"
) + ($javaFiles | ForEach-Object { "`"$($_.Replace('\', '/'))`"" })

[System.IO.File]::WriteAllLines($argFile, $argContent)

Write-Host "Compiling Java sources with JDK 17 javac..."
& $javac "@$argFile"
if ($LASTEXITCODE -ne 0) {
    Write-Error "javac compilation failed with exit code $LASTEXITCODE"
    exit 1
}

Write-Host "Copying resources..."
Copy-Item -Path "$srcResDir\*" -Destination $classesDir -Recurse -Force

Write-Host "Packaging named JAR..."
$namedJar = Join-Path $buildDir "mcgm-bridge-named.jar"
if (Test-Path $namedJar) {
    Remove-Item -Force $namedJar
}
Push-Location $classesDir
try {
    & $jar cf $namedJar *
} finally {
    Pop-Location
}

Write-Host "Remapping from named to intermediary namespace using tiny-remapper..."
$finalJar = Join-Path $libsDir "mcgm-bridge-1.0.0.jar"
if (Test-Path $finalJar) {
    Remove-Item -Force $finalJar
}

& $java -jar $tinyRemapperJar $namedJar $finalJar $yarnTiny named intermediary $namedMc
if ($LASTEXITCODE -ne 0) {
    Write-Error "tiny-remapper failed with exit code $LASTEXITCODE"
    exit 1
}

Write-Host "Built final mod JAR: $finalJar"
$finalItem = Get-Item $finalJar
Write-Host "Size: $($finalItem.Length) bytes"

$dotMinecraftDir = Split-Path $modsDir
if (Test-Path $dotMinecraftDir) {
    if (-not (Test-Path $modsDir)) {
        New-Item -ItemType Directory -Path $modsDir -Force | Out-Null
    }
    Write-Host "Installing to $modsDir\mcgm-bridge-1.0.0.jar..."
    Copy-Item -Path $finalJar -Destination (Join-Path $modsDir "mcgm-bridge-1.0.0.jar") -Force
} else {
    Write-Host "Skipping default .minecraft\mods install ($dotMinecraftDir not found)."
}

Write-Host "=========================================================="
Write-Host " Build & Installation complete!                           "
Write-Host "=========================================================="
