# Empaqueta Lumen Video Editor para Windows: dist\LumenVideo-<version>-windows-<arch>.zip (portable)
# y, con -Instalador, dist\LumenVideo-Setup-<version>-<arch>.exe y dist\latest.json.
#
#   powershell -ExecutionPolicy Bypass -File packaging\lumen\empaquetar.ps1                 # x64, compila si hace falta
#   powershell -ExecutionPolicy Bypass -File packaging\lumen\empaquetar.ps1 -Arch arm64     # ARM64 (herramientas ARM64 de MSVC)
#   ... -NoBuild      # empaqueta el .exe ya compilado sin llamar a cargo
#   ... -Instalador   # tambien el instalador (Inno Setup 6) y el aviso de actualizacion latest.json
#
# El paquete lleva LumenVideo.exe, los terminos de SifG22, las licencias (obligatorias: codigo MIT
# y componentes de terceros) y un LEEME.

param(
    [ValidateSet("x64", "arm64")] [string]$Arch = "x64",
    [switch]$NoBuild,
    [switch]$Instalador,
    # ISCC.exe de Inno Setup, si no esta en su sitio habitual.
    [string]$Iscc = "",
    # Donde se publicaran los instaladores (latest.json apunta ahi). Por defecto, la Release de
    # GitHub de esta version.
    [string]$BaseUrl = "",
    # Novedades que muestra el aviso de actualizacion.
    [string]$Notas = ""
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$version = (Select-String -Path "$root\crates\ui-egui\src\brand.rs" -Pattern 'pub const VERSION: &str = "([^"]+)"').Matches[0].Groups[1].Value
if (-not $version) { throw "No encuentro VERSION en crates\ui-egui\src\brand.rs" }
if (-not $BaseUrl) { $BaseUrl = "https://github.com/SifG22/lumen_video_editor/releases/download/v$version" }
$triple = if ($Arch -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$root\target" }

if (-not $NoBuild) {
    Push-Location $root
    try {
        # CRT estatico: no hace falta el redistribuible de Visual C++.
        $flagVar = 'CARGO_TARGET_' + ($triple.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
        [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
        $env:LUMEN_REQUIRE_WINRES = "1"
        if (-not $env:LUMEN_BUILD_SHA) { $env:LUMEN_BUILD_SHA = (git -C $root rev-parse HEAD 2>$null) }
        if (-not $env:LUMEN_BUILD_DATE) { $env:LUMEN_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd') }
        cargo build --release --locked -p lumen --target $triple
        if ($LASTEXITCODE -ne 0) { throw "La compilacion ha fallado" }
    } finally { Pop-Location }
}

$exe = "$targetDir\$triple\release\lumen.exe"
if (-not (Test-Path $exe)) { throw "No encuentro $exe; compila primero" }

# El .exe debe ser de la arquitectura pedida y de interfaz grafica (sin ventana de consola).
$bytes = [System.IO.File]::ReadAllBytes($exe)
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
$machine = [BitConverter]::ToUInt16($bytes, $pe + 4)
$subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C)
$expected = if ($Arch -eq "arm64") { 0xAA64 } else { 0x8664 }
if ($machine -ne $expected) { throw ("lumen.exe es para la maquina 0x{0:X}, no para {1}" -f $machine, $Arch) }
if ($subsystem -ne 2) { throw "lumen.exe no es una aplicacion grafica (subsistema $subsystem)" }

$name = "LumenVideo-$version-windows-$Arch"
$stage = "$root\dist\$name"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\licencias\terceros" | Out-Null
Copy-Item $exe "$stage\LumenVideo.exe"
Copy-Item "$root\LICENSE-MIT", "$root\LICENSE-APACHE", "$root\NOTICE", "$root\ATTRIBUTION.md" "$stage\licencias\"
Copy-Item "$root\assets\legal\TERMINOS.txt" "$stage\TERMINOS.txt"
# Licencias de terceros que van dentro de LumenVideo.exe (fuentes OFL, traducciones, icono).
foreach ($f in @("assets\fonts\OFL-Inter.txt", "assets\fonts\OFL-NotoSerif.txt", "assets\fonts\OFL-JetBrainsMono.txt", "assets\app-icon\LICENSE.txt")) {
    $leaf = (Split-Path $f -Leaf) -replace '^LICENSE\.txt$', 'LICENSE-icono-Lumen-Video.txt'
    if (Test-Path "$root\$f") { Copy-Item "$root\$f" "$stage\licencias\terceros\$leaf" }
}
foreach ($f in Get-ChildItem "$root\crates\ui-egui\src\i18n\*.attribution" -ErrorAction SilentlyContinue) {
    Copy-Item $f.FullName "$stage\licencias\terceros\traduccion-$($f.Name -replace '\.tsv\.attribution$', '.txt')"
}
# Fuentes japonesas de craft-fonts (compilaciones con CRAFT_FONTS_DIR) con su licencia OFL.
if ($env:CRAFT_FONTS_DIR) {
    foreach ($lic in Get-ChildItem -Path (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt') -ErrorAction SilentlyContinue) {
        Copy-Item $lic.FullName "$stage\licencias\terceros\OFL-$($lic.Directory.Name).txt"
    }
}
@"
Lumen Video Editor $version para Windows ($Arch), por SifG22 - https://sifg.net

Para empezar: abre LumenVideo.exe. No hace falta instalar nada.
Si Windows muestra "Windows protegio su PC", pulsa "Mas informacion" y "Ejecutar de todas formas"
(sale porque el programa aun no esta firmado digitalmente).

Version portable: si creas un archivo vacio llamado portable.txt junto a LumenVideo.exe, las
preferencias, ajustes y registros se guardan en la carpeta "data" de al lado (util en un USB).

Los proyectos se guardan como .lumenv. Tambien abre los proyectos .fcproj.

Al usar Lumen Video aceptas los terminos y condiciones de SifG22 (TERMINOS.txt).
Lumen Video incluye software de codigo abierto bajo la licencia MIT. Las licencias estan en la
carpeta "licencias".
"@ | Out-File -Encoding utf8 "$stage\LEEME.txt"

$zip = "$root\dist\$name.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path "$stage\*" -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $name.zip" | Out-File -Encoding ascii "$zip.sha256"
Write-Host "Listo: $zip"
Write-Host "SHA-256: $hash"

if ($Instalador) {
    $candidates = @($Iscc, "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
        Where-Object { $_ -and (Test-Path $_) }
    $compiler = $candidates | Select-Object -First 1
    if (-not $compiler) { throw "No encuentro ISCC.exe (Inno Setup 6); pasa -Iscc <ruta>" }
    & $compiler "/DVersion=$version" "/DArch=$Arch" "/DSource=$stage" "/DOutDir=$root\dist" "$PSScriptRoot\lumen-video.iss"
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup ha fallado" }
    $setup = "$root\dist\LumenVideo-Setup-$version-$Arch.exe"
    $setupHash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
    "$setupHash  LumenVideo-Setup-$version-$Arch.exe" | Out-File -Encoding ascii "$setup.sha256"
    Write-Host "Instalador: $setup"
    Write-Host "SHA-256: $setupHash"

    # latest.json: una entrada por arquitectura; la otra se conserva si es de la misma version.
    $manifestPath = "$root\dist\latest.json"
    $manifest = [ordered]@{ version = $version; notes = $Notas }
    if (Test-Path $manifestPath) {
        $old = Get-Content $manifestPath -Raw | ConvertFrom-Json
        if ($old.version -eq $version) {
            if (-not $Notas -and $old.notes) { $manifest.notes = $old.notes }
            foreach ($a in @("x64", "arm64")) { if ($a -ne $Arch -and $old.$a) { $manifest[$a] = $old.$a } }
        }
    }
    $manifest[$Arch] = [ordered]@{ url = "$BaseUrl/LumenVideo-Setup-$version-$Arch.exe"; sha256 = $setupHash }
    # Sin BOM: es JSON para otro programa.
    [System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding $false))
    Write-Host "Aviso de actualizacion: $manifestPath (va adjunto a la Release de GitHub; Lumen Video lo lee de alli)"
}
