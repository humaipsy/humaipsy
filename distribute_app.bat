@echo off
setlocal

set "ROOT_DIR=%~dp0"
set "APP_DIR=%ROOT_DIR%apps\humai"
set "RELEASE_DIR=%APP_DIR%\target\release"
set "DIST_DIR=%USERPROFILE%\Desktop\humai"
set "SIGN_CONFIG=%ROOT_DIR%signing\windows-signing.env"
set "SIGN_SCRIPT=%ROOT_DIR%scripts\sign_windows_release.ps1"

echo [1/3] Building release binary...
cd /d "%APP_DIR%" || exit /b 1
cargo build --release || exit /b 1

echo [2/3] Preparing distribution folder at %DIST_DIR% ...
if exist "%DIST_DIR%" (
    rmdir /s /q "%DIST_DIR%"
)
mkdir "%DIST_DIR%" || exit /b 1

echo [3/3] Copying release files...
copy "%RELEASE_DIR%\humai.exe" "%DIST_DIR%\humai.exe" || exit /b 1
if exist "%APP_DIR%\assets" (
    xcopy /e /i /q "%APP_DIR%\assets" "%DIST_DIR%\assets" || exit /b 1
)
if exist "%APP_DIR%\vault\.master_hash" (
    if not exist "%DIST_DIR%\vault" mkdir "%DIST_DIR%\vault" || exit /b 1
    copy "%APP_DIR%\vault\.master_hash" "%DIST_DIR%\vault\.master_hash" || exit /b 1
) else (
    echo WARNING: %APP_DIR%\vault\.master_hash not found. Distribution will stay locked until hash is provided.
)

if exist "%SIGN_CONFIG%" (
    echo [sign] Signing enabled via %SIGN_CONFIG% ...
    powershell -NoProfile -ExecutionPolicy Bypass -File "%SIGN_SCRIPT%" -FilePath "%DIST_DIR%\humai.exe" -ConfigPath "%SIGN_CONFIG%" || exit /b 1
) else (
    echo [sign] Signing skipped. Create %SIGN_CONFIG% from signing\windows-signing.env.example to enable automatic signing.
)

echo.
echo Distribution complete: %DIST_DIR%
pause
