@echo off
rem FrisyDisk in the terminal. Works from PowerShell and Command Prompt.
setlocal
rem Find programs on PATH only, never in the current folder.
set "NoDefaultCurrentDirectoryInExePath=1"
set "FRISYSCAN=%~dp0frisyscan.exe"
if /I "%~1"=="du" (
  "%FRISYSCAN%" %*
  exit /b %ERRORLEVEL%
)
where node >nul 2>nul
if errorlevel 1 goto nonode
node -e "process.exit(+process.versions.node.split('.')[0] >= 18 ? 0 : 1)" >nul 2>nul
if errorlevel 1 goto nonode
node "%~dp0frisy.mjs" %*
exit /b %ERRORLEVEL%

:nonode
echo frisy: the interactive view needs Node.js 18 or newer ^(winget install OpenJS.NodeJS.LTS^). Showing a summary instead. 1>&2
if "%~1"=="" ( "%FRISYSCAN%" . --top 25 ) else ( "%FRISYSCAN%" "%~1" --top 25 )
exit /b %ERRORLEVEL%
