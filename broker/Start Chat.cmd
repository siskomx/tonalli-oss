@echo off
cd /d "%~dp0"
"%~dp0node.exe" "%~dp0src\launch.mjs"
if errorlevel 1 pause
