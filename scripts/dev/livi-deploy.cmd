@echo off
setlocal
rem Builds the arm64 binaries in WSL and deploys them to the device.
rem Pass options through, e.g.: livi-deploy.cmd --host 192.168.0.128 --no-restart

for %%I in ("%~dp0..\..") do set "REPO=%%~fI"

where wsl.exe >nul 2>&1
if errorlevel 1 (
  echo Error: wsl.exe not found. Install WSL2 first: wsl --install -d Debian
  exit /b 1
)

wsl.exe --cd "%REPO%" -e bash ./scripts/dev/livi-deploy.sh %*
exit /b %ERRORLEVEL%
