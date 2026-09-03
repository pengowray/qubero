@echo off
rem Build the site into web\dist, the same steps the deploy workflow runs.
rem The POSIX twin is build.sh.
rem
rem Unlike run.bat this rebuilds the wasm every time: a release build should
rem not ship whatever the last dev session happened to leave behind.
rem
rem Every npm call needs `call`: npm is a .cmd, and a batch file that runs
rem another one without it hands over and never comes back, so the line after
rem would not run.

setlocal
cd /d "%~dp0web" || exit /b 1

where wasm-pack >nul 2>nul || (
  echo wasm-pack is not installed. Run: cargo install wasm-pack
  exit /b 1
)
rustup target list --installed 2>nul | findstr /c:"wasm32-unknown-unknown" >nul || (
  echo The wasm target is missing. Run: rustup target add wasm32-unknown-unknown
  exit /b 1
)

echo Installing dependencies...
call npm install || exit /b 1

echo Building wasm...
call npm run wasm || exit /b 1

rem Pinned to one commit, so an existing download is the right one.
if not exist "public\diesig\pe.sig" (
  echo Downloading signatures...
  node ..\tools\die.mjs || exit /b 1
)

echo Building site...
call npm run build || exit /b 1

echo.
echo Built web\dist. To serve it: npm run preview in web
