@echo off
setlocal

set "ROOT_DIR=%~dp0"
set "APP_DIR=%ROOT_DIR%apps\humai"
set "RELEASE_DIR=%APP_DIR%\target\release"
set "DIST_DIR=%USERPROFILE%\Desktop\humai"

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

echo.
echo Distribution complete: %DIST_DIR%
pause
