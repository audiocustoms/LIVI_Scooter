@echo off
setlocal
rem Starts the LIVI UI in a window on this PC. Everything happens inside WSL2.
rem Pass options through, e.g.: livi-window.cmd --page settings --settings car/carType

for %%I in ("%~dp0..\..") do set "REPO=%%~fI"

where wsl.exe >nul 2>&1
if errorlevel 1 (
  echo Error: wsl.exe not found. Install WSL2 first: wsl --install -d Debian
  exit /b 1
)

wsl.exe --cd "%REPO%" -e bash ./scripts/dev/livi-window.sh %*
exit /b %ERRORLEVEL%
