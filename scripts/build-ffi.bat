@echo off
setlocal EnableExtensions

set "SCRIPT_DIR=%~dp0"
for %%I in ("%SCRIPT_DIR%..") do set "REPO_ROOT=%%~fI"

set "DIST_DIR=%REPO_ROOT%\dist\badness-ffi"
set "INCLUDE_DIR=%DIST_DIR%\inc\badness"
set "LIB_DIR=%DIST_DIR%\libs"
set "LIB_SOURCE_DIR=%REPO_ROOT%\target\release"

where cargo >nul 2>nul
if errorlevel 1 (
    echo cargo not found in PATH 1>&2
    exit /b 1
)

if not exist "%INCLUDE_DIR%" mkdir "%INCLUDE_DIR%"
if not exist "%LIB_DIR%" mkdir "%LIB_DIR%"

cargo build --manifest-path "%REPO_ROOT%\Cargo.toml" --release --lib --features compact-data

copy /Y "%REPO_ROOT%\include\badness_ffi.h" "%INCLUDE_DIR%\badness_ffi.h" >nul
copy /Y "%REPO_ROOT%\include\badness.hpp" "%INCLUDE_DIR%\badness.hpp" >nul
copy /Y "%LIB_SOURCE_DIR%\badness.dll" "%LIB_DIR%\badness.dll" >nul
copy /Y "%LIB_SOURCE_DIR%\badness.dll.lib" "%LIB_DIR%\badness.lib" >nul

dir /b /s "%DIST_DIR%"
