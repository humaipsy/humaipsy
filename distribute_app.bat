@echo off
setlocal

set "ROOT_DIR=%~dp0"
set "APP_DIR=%ROOT_DIR%apps\humaipsy"
set "RELEASE_DIR=%APP_DIR%\target\release"
set "DIST_DIR=%USERPROFILE%\Desktop\humaipsy"
if defined HUMAIPSY_DIST_DIR set "DIST_DIR=%HUMAIPSY_DIST_DIR%"

echo [1/3] Building release binary...
cd /d "%APP_DIR%" || exit /b 1
cargo build --release || exit /b 1

echo [2/3] Preparing distribution folder at %DIST_DIR% ...
if exist "%DIST_DIR%" (
    rmdir /s /q "%DIST_DIR%"
)
mkdir "%DIST_DIR%" || exit /b 1

echo [3/3] Copying release files...
copy "%RELEASE_DIR%\humaipsy.exe" "%DIST_DIR%\humaipsy.exe" || exit /b 1
if exist "%APP_DIR%\assets" (
    xcopy /e /i /q "%APP_DIR%\assets" "%DIST_DIR%\assets" || exit /b 1
)
if exist "%APP_DIR%\vault\.master_hash" (
    if not exist "%DIST_DIR%\vault" mkdir "%DIST_DIR%\vault" || exit /b 1
    copy "%APP_DIR%\vault\.master_hash" "%DIST_DIR%\vault\.master_hash" || exit /b 1
) else (
    echo WARNING: %APP_DIR%\vault\.master_hash not found. Distribution will stay locked until hash is provided.
)

echo.
echo Distribution complete: %DIST_DIR%
pause
